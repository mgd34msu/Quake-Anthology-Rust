use super::*;
use qa_app::modules::{ModuleResult, Q3Spec, State, load_q3};
use qa_core::primitives::{ModuleId, ThinkTime};
use qa_formats::program::qvm::Opcode;
use qa_network::{
    channel::commands::{CommandContext, write_gamestate},
    commands::packet::{self, Move},
    message::{AdaptiveHuffman, Writer},
    snapshots::{Frame, ReceivedFrame},
    states,
};

#[path = "../support/service_program.rs"]
#[allow(dead_code)]
mod program;
#[path = "../support/sound_registration.rs"]
#[allow(dead_code)]
mod samples;

const Q3: Protocol = Protocol::Quake3_68;
const CHALLENGE: i32 = -19;
const SERVER_ID: i32 = 731;
const FEED: u32 = 0x1234_5678;

fn context() -> Context {
    Context {
        source: RuleSetId::Quake3,
        ..Default::default()
    }
}
fn setting(host: &mut FrameHost, name: &str, value: &str) {
    let view = host.console.cvars.bind(name, context()).unwrap();
    host.console.cvars.force_write(view, value).unwrap();
}
fn connect(
    host: &mut FrameHost,
    source: &mut Source,
    seat: SeatId,
    address: SocketAddr,
    policy: Option<ClientPolicy>,
    time: u64,
) -> ClientId {
    host.console
        .append(
            &format!("connect {address}\n"),
            Context {
                seat,
                client_policy: policy,
                ..context()
            },
        )
        .unwrap();
    frame(host, source, time);
    host.local_clients[seat.index()].unwrap()
}
fn info(host: &FrameHost) -> Vec<u8> {
    let mut bytes = [0; 1024];
    let length = host
        .console
        .cvars
        .info_string(context(), 2, Q3.userinfo_rules().unwrap(), &mut bytes)
        .unwrap();
    bytes[..length].to_vec()
}
fn connect_info(packet: &[u8]) -> Vec<u8> {
    assert_eq!(&packet[..12], b"\xff\xff\xff\xffconnect ");
    let mut decoded = [0; 1024];
    let length = AdaptiveHuffman::new()
        .decompress(&packet[12..], &mut decoded)
        .unwrap();
    assert_eq!(decoded[0], b'"');
    assert_eq!(decoded[length - 1], b'"');
    decoded[1..length - 1].to_vec()
}
fn field<'a>(info: &'a [u8], key: &[u8]) -> &'a [u8] {
    &info[qa_core::text::info_value(info, key, false).unwrap()]
}
fn native(qport: u16) -> NativePeer {
    let mut native = NativePeer::new(Q3, qport);
    native
        .channel
        .set_command_context(CommandContext {
            challenge: CHALLENGE as u32,
            ..Default::default()
        })
        .unwrap();
    native
}
fn collect(server: &mut EventPump, native: &mut NativePeer, time: u64) {
    for (_, bytes) in datagrams(server) {
        assert!(!bytes.starts_with(&[255; 4]));
        native.receive(&bytes, time);
    }
}
fn accept(
    host: &mut FrameHost,
    source: &mut Source,
    seat: SeatId,
    server: &mut EventPump,
    socket: u16,
    local: SocketAddr,
    start: u64,
) -> NativePeer {
    response(server, socket, local, b"challengeResponse -19");
    frame(host, source, start + 10);
    let packets = datagrams(server);
    assert_eq!(packets.len(), 1);
    assert_eq!(field(&connect_info(&packets[0].1), b"challenge"), b"-19");
    response(server, socket, local, b"connectResponse");
    frame(host, source, start + 20);
    let qport = host
        .runtime
        .network
        .get(host.local_clients[seat.index()].unwrap(), Endpoint::Client)
        .unwrap()
        .channel
        .qport();
    let mut native = native(qport);
    collect(server, &mut native, start + 20);
    assert_eq!(native.count(b"new"), 0);
    assert!(native.q3_moves.is_empty());
    native
}

#[test]
fn q3_udp_admission_retries_compressed_current_info_and_captures_proxy_and_accept_ports() {
    let mut original = EventPump::new();
    let (original_socket, address) = original.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let mut proxy = EventPump::new();
    let (proxy_socket, proxy_address) = proxy.bind_udp(([127, 0, 0, 2], 0).into()).unwrap();
    let mut accepted = EventPump::new();
    let (accepted_socket, accepted_address) =
        accepted.bind_udp(([127, 0, 0, 2], 0).into()).unwrap();
    let mut source = source();
    let mut host = host(2);
    setting(&mut host, "net_qport", "65535");
    setting(&mut host, "name", "Original");
    let policy = ClientPolicy {
        client: RuleSetId::Quake2,
        input: RuleSetId::Quake3,
        movement: RuleSetId::Quake,
        trace: RuleSetId::QuakeWorld,
    };
    let client = connect(
        &mut host,
        &mut source,
        SeatId::FIRST,
        address,
        Some(policy),
        0,
    );
    let packets = datagrams(&mut original);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge");
    let local = peer(&packets[0]);
    let old_token = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .route
        .socket;
    let old_entity = host.runtime.server.clients[client.0 as usize].entity;
    frame(&mut host, &mut source, 2999);
    assert!(datagrams(&mut original).is_empty());
    frame(&mut host, &mut source, 3000);
    let packets = datagrams(&mut original);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge");
    source.reject_connect = true;
    response(&mut proxy, proxy_socket, local, b"challengeResponse -19");
    frame(&mut host, &mut source, 3001);
    assert_eq!(source.rejected_connects, 1);
    assert!(datagrams(&mut proxy).is_empty());
    setting(&mut host, "name", "Actually admitted");
    source.reject_connect = false;
    frame(&mut host, &mut source, 3002);
    let packets = datagrams(&mut proxy);
    assert_eq!(packets.len(), 1);
    let admitted_info = connect_info(&packets[0].1);
    assert_eq!(field(&admitted_info, b"protocol"), b"68");
    assert_eq!(field(&admitted_info, b"qport"), b"65535");
    assert_eq!(field(&admitted_info, b"challenge"), b"-19");
    assert_eq!(field(&admitted_info, b"name"), b"Actually admitted");
    assert_eq!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .route
            .peer,
        Peer::Socket(proxy_address)
    );
    frame(&mut host, &mut source, 6001);
    assert!(datagrams(&mut proxy).is_empty());
    setting(&mut host, "name", "Retry admitted");
    frame(&mut host, &mut source, 6002);
    let packets = datagrams(&mut proxy);
    assert_eq!(packets.len(), 1);
    assert_eq!(
        field(&connect_info(&packets[0].1), b"name"),
        b"Retry admitted"
    );
    response(&mut original, original_socket, local, b"connectResponse");
    frame(&mut host, &mut source, 6003);
    assert!(matches!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .phase,
        ConnectionPhase::Connecting(_)
    ));
    setting(&mut host, "name", "Changed after send");
    response(&mut accepted, accepted_socket, local, b"connectResponse");
    frame(&mut host, &mut source, 6004);
    let connection = host.runtime.network.get(client, Endpoint::Client).unwrap();
    assert!(connection.phase.is_connected());
    assert_eq!(connection.route.peer, Peer::Socket(accepted_address));
    assert_eq!(
        connection.channel.command_context().unwrap().challenge,
        CHALLENGE as u32
    );
    let mut native = native(65535);
    collect(&mut accepted, &mut native, 6004);
    assert!(native.q3_moves.is_empty());
    assert_eq!(history(&host, client), 0);
    let mut update = b"userinfo \"".to_vec();
    update.extend(info(&host));
    update.push(b'"');
    assert_eq!(native.count(&update), 1);
    assert_eq!(native.count(b"new"), 0);
    host.console.append("reconnect\n", context()).unwrap();
    frame(&mut host, &mut source, 6010);
    let packets = datagrams(&mut accepted);
    let requests: Vec<_> = packets
        .iter()
        .filter(|packet| packet.1.starts_with(&[255; 4]))
        .collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1, b"\xff\xff\xff\xffgetchallenge");
    let replacement = host.local_clients[0].unwrap();
    assert_ne!(
        host.runtime.server.clients[replacement.0 as usize].entity,
        old_entity
    );
    let connection = host
        .runtime
        .network
        .get(replacement, Endpoint::Client)
        .unwrap();
    assert_ne!(connection.route.socket, old_token);
    assert_eq!(connection.route.peer, Peer::Socket(accepted_address));
    assert_eq!(
        host.runtime.server.clients[replacement.0 as usize].client_rules,
        policy.client
    );
    assert_eq!(
        host.runtime.prediction[0].player.movement_rules,
        policy.movement
    );
    assert_eq!(host.runtime.prediction[0].player.trace_rules, policy.trace);
    assert!(source.pump.send_udp(old_token, address, b"closed").is_err());
}

fn cgame() -> Vec<u8> {
    use Opcode::*;
    let mut ops = vec![(Enter, 64), (Local, 72), (Load4, 0), (Const, 0), (Eq, 0)];
    let result = [
        (Const, 128),
        (Load4, 0),
        (Const, 100),
        (MulI, 0),
        (Const, 132),
        (Load4, 0),
        (Add, 0),
        (Leave, 64),
    ];
    ops.extend(result);
    ops[4].1 = ops.len() as i32;
    // A real CG_INIT records its native client argument and increments once.
    ops.extend([
        (Const, 160),
        (Arg, 8),
        (Const, -37), // CG_R_LOADWORLDMAP
        (Call, 0),
        (Pop, 0),
        (Const, 132),
        (Local, 84),
        (Load4, 0),
        (Store4, 0),
        (Const, 128),
        (Const, 128),
        (Load4, 0),
        (Const, 1),
        (Add, 0),
        (Store4, 0),
    ]);
    ops.extend(result);
    let mut data = [0; 256];
    data[160..175].copy_from_slice(b"maps/ready.bsp\0");
    program::bytes(&ops, &data)
}

fn media_runtime(
    vfs: qa_content::vfs::Vfs,
) -> (Box<Runtime>, Vec<qa_app::modules::ModuleRequest>, ClientId) {
    let mut runtime = Box::new(Runtime::load(3, []).unwrap());
    runtime.vfs = vfs;
    let old = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                client: RuleSetId::Quake3,
                input: RuleSetId::Quake3,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake3,
            },
            Q3,
            0,
        )
        .unwrap();
    let mut requests = vec![];
    load_q3(
        &mut runtime,
        &[Q3Spec::parse("cgame:0:vm/udp.qvm").unwrap()],
        &mut requests,
        TickRate::FrameDriven,
        None,
    )
    .unwrap();
    (runtime, requests, old)
}

fn media_assets(runtime: &mut Runtime) -> (Source, qa_app::model_assets::ModelProfile) {
    let mut assets = qa_render::Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile =
        qa_app::model_assets::ModelRequest::native("", RuleSetId::Quake3, Some(palette)).profile();
    let (scene, _) =
        qa_app::renderer::SceneComposer::load(Default::default(), &mut assets, vec![]).unwrap();
    let mut owner = qa_app::model_assets::ModelAssets::new(
        &runtime.vfs,
        assets,
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([Some(palette); 5]),
        )
        .unwrap();
    let backend = asset_frames::Source::load(owner.assets());
    runtime.model_assets = Some(owner);
    runtime.scene = Some(scene);
    runtime.sound_assets = Some(Box::new(
        qa_app::audio::SoundAssets::load(32, 4096).unwrap(),
    ));
    let mut source = source();
    source.backend = Some(backend);
    source.prepare_hud = true;
    (source, profile)
}

fn requested_host(
    runtime: Box<Runtime>,
    requests: Vec<qa_app::modules::ModuleRequest>,
) -> Box<FrameHost> {
    Box::new(
        FrameHost::load_modules(
            Console::new(context()).unwrap(),
            *runtime,
            TickRate::FrameDriven,
            requests,
        )
        .unwrap(),
    )
}

fn media_host() -> (
    support::Fixture,
    Box<FrameHost>,
    Source,
    qa_app::model_assets::ModelProfile,
) {
    let mut fixture = support::Fixture::new(&[
        ("maps/ready.bsp", support::world_map(16.0)),
        ("models/test/tris.md2", support::md2()),
        ("models/test/skin0.pcx", support::pcx(11)),
        ("models/test/skin1.pcx", support::pcx(22)),
        ("sound/step.wav", samples::wav()),
        ("vm/udp.qvm", cgame()),
        ("scripts/udp.shader", b"gfx/2d/bigchars { { map $whiteimage } }\nwhite { { map $whiteimage } }\nconsole { { map $whiteimage } }\n".to_vec()),
    ]);
    let (mut runtime, requests, old) = media_runtime(std::mem::take(&mut fixture.vfs));
    let (source, profile) = media_assets(&mut runtime);
    let mut host = requested_host(runtime, requests);
    host.local_clients[0] = Some(old);
    let mut defaults = [None; 5];
    defaults[RuleSetId::Quake3 as usize] = Some(profile);
    host.configure_native_scenes(&defaults, &[[None; 5]; SeatId::COUNT], 64, 48)
        .unwrap();
    host.publish_model_admission(
        source
            .backend
            .as_ref()
            .unwrap()
            .cpu
            .admitted_assets()
            .unwrap(),
    )
    .unwrap();
    host.runtime
        .model_assets
        .as_mut()
        .unwrap()
        .end_initial_registration();
    (fixture, host, source, profile)
}

fn send(
    server: &mut EventPump,
    socket: u16,
    local: SocketAddr,
    native: &mut NativePeer,
    time: u64,
    body: impl FnOnce(&mut Writer<'_>, &mut Channel) -> Result<(), packet::Error>,
) -> usize {
    let mut bytes = vec![0; Q3.message_bytes()];
    let length = native
        .channel
        .encode_server_output(&mut bytes, body)
        .unwrap();
    let at = EventTime::from_milliseconds(time);
    native
        .channel
        .prepare_server_message(&bytes[..length], at)
        .unwrap()
        .unwrap();
    let mut packets = 0;
    loop {
        let packet = native.channel.pending_packet().unwrap();
        server.send_udp(socket, local, packet.bytes).unwrap();
        native.channel.submitted(at).unwrap();
        packets += 1;
        if !native.channel.pending_fragments() {
            break;
        }
        native.channel.prepare_output(at).unwrap().unwrap();
    }
    packets
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GamestateContent {
    Ordinary,
    Pure,
    InvalidTail,
}

fn gamestate(
    server: &mut EventPump,
    socket: u16,
    local: SocketAddr,
    native: &mut NativePeer,
    time: u64,
    content: GamestateContent,
) -> usize {
    native
        .channel
        .set_command_context(CommandContext {
            server_id: SERVER_ID,
            challenge: CHALLENGE as u32,
            checksum_feed: FEED,
        })
        .unwrap();
    let mut rows = vec![
        (0, b"\\mapname\\ready".to_vec()),
        (
            1,
            format!(
                "\\sv_serverid\\{SERVER_ID}\\sv_pure\\{}",
                u8::from(content == GamestateContent::Pure)
            )
            .into_bytes(),
        ),
        (33, b"maps/ready.bsp".to_vec()),
        (34, b"models/test/tris.md2".to_vec()),
        (35, b"models/optional/absent.md3".to_vec()),
        (289, b"sound/step.wav".to_vec()),
        (290, b"sound/optional/absent.wav".to_vec()),
    ];
    for row in 700..712 {
        rows.push((
            row,
            (0..400)
                .map(|index| b'a' + ((index * 17 + row) % 26) as u8)
                .collect(),
        ));
    }
    let watermark = native.channel.command_state().unwrap().queued;
    send(server, socket, local, native, time, |writer, _| {
        write_gamestate(
            writer,
            watermark,
            4,
            FEED,
            rows.iter().map(|(index, text)| (*index, text.as_slice())),
            std::iter::empty(),
        )?;
        if content == GamestateContent::InvalidTail {
            writer.write_bits(255, 8)?;
        }
        Ok(())
    })
}

fn snapshot(
    server: &mut EventPump,
    socket: u16,
    local: SocketAddr,
    native: &mut NativePeer,
    time: u64,
    flags: u8,
) -> u32 {
    let sequence = native.channel.send_state().sequence;
    let mut player = [0; states::PLAYER_WORDS];
    player[0] = time as u32;
    player[1] = 32f32.to_bits();
    player[40] = 4; // Native playerState.clientNum, states::PLAYER_LAYOUT.
    let frame = ReceivedFrame::Quake3(Frame {
        sequence,
        time: ThinkTime::Milliseconds(time as i64),
        command: 0,
        flags,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &player,
        entities: &[],
    });
    native.channel.publish_snapshot(frame).unwrap();
    send(server, socket, local, native, time, |writer, channel| {
        // This shared writer reads the already-published snapshot's native row.
        channel.write_snapshot(writer, sequence, None, 0)
    });
    sequence
}

#[test]
fn q3_fragmented_gamestate_waits_for_real_media_backend_and_current_cgame_before_priming() {
    let (_fixture, mut host, mut source, profile) = media_host();
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    setting(&mut host, "net_qport", "17");
    let failed_client = connect(&mut host, &mut source, SeatId::FIRST, address, None, 0);
    let failed_lifetime = host.runtime.server.clients[failed_client.0 as usize].entity;
    let failed_socket = host
        .runtime
        .network
        .get(failed_client, Endpoint::Client)
        .unwrap()
        .route
        .socket;
    host.bind_client_model_assets([(failed_client, profile, None)]);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    let local = peer(&packets[0]);
    let mut native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        local,
        0,
    );
    assert_ne!(host.module_state(ModuleId(1)), Some(State::Running));
    let original = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    source.reject_assets = true;
    assert!(
        gamestate(
            &mut server,
            socket,
            local,
            &mut native,
            30,
            GamestateContent::Ordinary
        ) > 1
    );
    frame(&mut host, &mut source, 30);
    collect(&mut server, &mut native, 30);
    assert_eq!(
        host.runtime
            .network
            .get(failed_client, Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup()
            .map(|startup| (startup.client_num, startup.checksum_feed)),
        Some((4, FEED)),
        "the complete encrypted fragmented gamestate must precede media preparation"
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    frame(&mut host, &mut source, 31);
    collect(&mut server, &mut native, 31);
    assert!(source.rejected_assets > 0);
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        original
    );
    assert!(host.local_worlds[0].is_none());
    assert!(
        host.runtime
            .network
            .get(failed_client, Endpoint::Client)
            .is_none()
    );
    assert_eq!(host.local_clients[0], None);
    assert!(
        source
            .pump
            .send_udp(failed_socket, address, b"closed")
            .is_err()
    );
    assert!(native.q3_moves.is_empty());
    assert_ne!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    source.reject_assets = false;
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, None, 40);
    assert_ne!(
        host.runtime.server.clients[client.0 as usize].entity,
        failed_lifetime
    );
    host.bind_client_model_assets([(client, profile, None)]);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge");
    let local = peer(&packets[0]);
    native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        local,
        40,
    );
    assert!(
        gamestate(
            &mut server,
            socket,
            local,
            &mut native,
            70,
            GamestateContent::Ordinary
        ) > 1
    );
    frame(&mut host, &mut source, 70);
    collect(&mut server, &mut native, 70);
    frame(&mut host, &mut source, 80);
    collect(&mut server, &mut native, 80);
    frame(&mut host, &mut source, 85);
    collect(&mut server, &mut native, 85);
    assert!(host.local_worlds[0].is_some());
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(104)),
        "one CG_INIT must capture native client4 after media admission"
    );
    assert_eq!(
        source.backend.as_ref().unwrap().cpu.admitted_assets(),
        Some(
            host.runtime
                .model_assets
                .as_ref()
                .unwrap()
                .assets()
                .counts()
        )
    );
    {
        use qa_core::primitives::{ModelRef, ResourceNamespace};
        let owner = host.runtime.model_assets.as_ref().unwrap();
        let resources = &host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .state;
        let resolve = |ordinal| {
            resources.models.resolve(
                ModelRef::native(ResourceNamespace::Client(client), ordinal).unwrap(),
                0,
                owner.catalog(),
                Default::default(),
            )
        };
        assert!(
            resolve(2).is_some(),
            "the valid MD2 must have admitted materials"
        );
        assert!(
            resolve(3).is_none(),
            "the optional missing model keeps native zero fallback"
        );
        let bank = host.runtime.sound_bank().unwrap();
        assert!(bank.sample(resources.sound(1).unwrap()).is_some());
        assert_eq!(
            resources.sound(2),
            None,
            "the optional missing sound keeps native zero fallback"
        );
    }
    let Move::Quake3 {
        commands,
        count,
        delta,
    } = native.q3_moves[0]
    else {
        panic!("initial Q3 move")
    };
    assert!(!delta);
    assert_eq!(count, 1);
    assert_eq!(commands[0].server_time, 0);
    assert_eq!(commands[0].movement, [0; 3]);
    assert_eq!(
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .native_client(),
        Some(4)
    );
    host.console.append("+forward\n", context()).unwrap();
    snapshot(&mut server, socket, local, &mut native, 90, 2);
    frame(&mut host, &mut source, 90);
    collect(&mut server, &mut native, 90);
    assert!(
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .interpolation()
            .is_none()
    );
    assert_eq!(native.moves, 0);
    snapshot(&mut server, socket, local, &mut native, 100, 0);
    frame(&mut host, &mut source, 100);
    collect(&mut server, &mut native, 100);
    frame(&mut host, &mut source, 110);
    collect(&mut server, &mut native, 110);
    assert!(native.moves > 0);
    assert!(history(&host, client) > 0);
    assert!(native.q3_moves.iter().any(|movement| matches!(movement,
        Move::Quake3 { commands, count, .. }
            if commands[..usize::from(*count)].iter().any(|command| command.movement[0] > 0)
    )));

    let applied = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .applied;
    let active_not_active = snapshot(&mut server, socket, local, &mut native, 115, 2);
    frame(&mut host, &mut source, 115);
    collect(&mut server, &mut native, 115);
    let binding = host.runtime.snapshots[client.0 as usize].as_ref().unwrap();
    assert_eq!(
        binding.applied,
        applied + 1,
        "SNAPFLAG_NOT_ACTIVE only guards first activation"
    );
    let ReceivedFrame::Quake3(received) = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .snapshot(active_not_active)
        .unwrap()
    else {
        panic!("native Q3 CLIENT snapshot")
    };
    assert_eq!(received.flags, 2);
    assert_eq!(received.player[0], 115); // Native commandTime.
    assert_eq!(received.player[1], 32f32.to_bits()); // Native origin[0].
    assert_eq!(received.player[40], 4); // Native clientNum.

    let old_lifetime = host.runtime.server.clients[client.0 as usize].entity;
    host.console.append("reconnect\n", context()).unwrap();
    frame(&mut host, &mut source, 120);
    let packets = datagrams(&mut server);
    let request: Vec<_> = packets
        .iter()
        .filter(|(_, bytes)| bytes.starts_with(&[255; 4]))
        .collect();
    assert_eq!(request.len(), 1);
    assert_eq!(request[0].1, b"\xff\xff\xff\xffgetchallenge");
    let new_local = peer(request[0]);
    assert_ne!(new_local, local);
    let replacement = host.local_clients[0].unwrap();
    assert_ne!(
        host.runtime.server.clients[replacement.0 as usize].entity,
        old_lifetime
    );
    assert_eq!(history(&host, replacement), 0);
    assert_ne!(host.module_state(ModuleId(1)), Some(State::Running));
    host.bind_client_model_assets([(replacement, profile, None)]);
    native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        new_local,
        120,
    );
    assert!(
        gamestate(
            &mut server,
            socket,
            new_local,
            &mut native,
            150,
            GamestateContent::Ordinary
        ) > 1
    );
    frame(&mut host, &mut source, 150);
    collect(&mut server, &mut native, 150);
    frame(&mut host, &mut source, 160);
    collect(&mut server, &mut native, 160);
    frame(&mut host, &mut source, 165);
    collect(&mut server, &mut native, 165);
    assert!(host.local_worlds[0].is_some());
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(104))
    );
    assert!(!native.q3_moves.is_empty());
    assert_eq!(native.moves, 0);
    assert!(
        matches!(native.q3_moves[0], Move::Quake3 { commands, delta: false, .. }
        if commands[0].server_time == 0 && commands[0].movement == [0; 3])
    );
}

#[test]
fn q3_invalid_service_after_gamestate_drops_only_its_client_before_init() {
    let (_fixture, mut host, mut source, profile) = media_host();
    let seat = SeatId::new(1).unwrap();
    let old = host.local_clients[0].unwrap();
    let mut healthy_server = EventPump::new();
    let (healthy_socket, healthy_address) =
        healthy_server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let healthy = connect(&mut host, &mut source, seat, healthy_address, None, 0);
    host.bind_client_model_assets([(old, profile, None), (healthy, profile, None)]);
    let packets = datagrams(&mut healthy_server);
    assert_eq!(packets.len(), 1);
    let healthy_local = peer(&packets[0]);
    let mut healthy_native = accept(
        &mut host,
        &mut source,
        seat,
        &mut healthy_server,
        healthy_socket,
        healthy_local,
        0,
    );
    assert!(
        gamestate(
            &mut healthy_server,
            healthy_socket,
            healthy_local,
            &mut healthy_native,
            30,
            GamestateContent::Ordinary,
        ) > 1
    );
    for time in [30, 40, 45] {
        frame(&mut host, &mut source, time);
        collect(&mut healthy_server, &mut healthy_native, time);
    }
    assert!(host.local_worlds[seat.index()].is_some());
    assert!(!healthy_native.q3_moves.is_empty());
    snapshot(
        &mut healthy_server,
        healthy_socket,
        healthy_local,
        &mut healthy_native,
        50,
        0,
    );
    for time in [50, 55] {
        frame(&mut host, &mut source, time);
        collect(&mut healthy_server, &mut healthy_native, time);
    }
    assert!(healthy_native.moves > 0);
    assert!(history(&host, healthy) > 0);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);

    let mut bad_server = EventPump::new();
    let (bad_socket, bad_address) = bad_server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let bad = connect(&mut host, &mut source, SeatId::FIRST, bad_address, None, 60);
    let owned_socket = host
        .runtime
        .network
        .get(bad, Endpoint::Client)
        .unwrap()
        .route
        .socket;
    host.bind_client_model_assets([
        (bad, profile, None),
        (healthy, profile, host.local_worlds[seat.index()]),
    ]);
    let packets = datagrams(&mut bad_server);
    assert_eq!(packets.len(), 1);
    let bad_local = peer(&packets[0]);
    let mut bad_native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut bad_server,
        bad_socket,
        bad_local,
        60,
    );
    collect(&mut healthy_server, &mut healthy_native, 80);
    let assets = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    let healthy_lifetime = host.runtime.server.clients[healthy.0 as usize].entity;
    let applied = host.runtime.snapshots[healthy.0 as usize]
        .as_ref()
        .unwrap()
        .applied;
    let before_history = history(&host, healthy);
    let before_moves = healthy_native.moves;
    let decode_errors = host.runtime.network.command_errors;

    assert!(
        gamestate(
            &mut bad_server,
            bad_socket,
            bad_local,
            &mut bad_native,
            90,
            GamestateContent::InvalidTail,
        ) > 1
    );
    snapshot(
        &mut healthy_server,
        healthy_socket,
        healthy_local,
        &mut healthy_native,
        90,
        0,
    );
    for time in [90, 100] {
        frame(&mut host, &mut source, time);
        collect(&mut bad_server, &mut bad_native, time);
        collect(&mut healthy_server, &mut healthy_native, time);
    }
    assert_eq!(host.local_clients[0], None);
    assert_eq!(host.runtime.network.command_errors, decode_errors + 1);
    assert!(host.runtime.network.get(bad, Endpoint::Client).is_none());
    assert!(host.local_worlds[0].is_none());
    assert!(
        source
            .pump
            .send_udp(owned_socket, bad_address, b"closed")
            .is_err()
    );
    assert!(bad_native.q3_moves.is_empty());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    assert_ne!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        assets
    );
    assert_eq!(host.local_clients[seat.index()], Some(healthy));
    assert_eq!(
        host.runtime.server.clients[healthy.0 as usize].entity,
        healthy_lifetime
    );
    assert!(
        host.runtime
            .network
            .get(healthy, Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
    assert!(host.local_worlds[seat.index()].is_some());
    assert_eq!(
        host.runtime.snapshots[healthy.0 as usize]
            .as_ref()
            .unwrap()
            .applied,
        applied + 1
    );
    assert!(history(&host, healthy) > before_history);
    assert!(healthy_native.moves > before_moves);
    assert!(!host.runtime.quit);
}

#[test]
fn q3_pure_gamestate_is_rejected_before_media_or_priming() {
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let mut source = source();
    let mut host = host(1);
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, None, 0);
    let packets = datagrams(&mut server);
    let local = peer(&packets[0]);
    let mut native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        local,
        0,
    );
    assert!(
        gamestate(
            &mut server,
            socket,
            local,
            &mut native,
            30,
            GamestateContent::Pure
        ) > 1
    );
    frame(&mut host, &mut source, 30);
    assert!(host.runtime.network.get(client, Endpoint::Client).is_none());
    assert_eq!(host.local_clients[0], None);
    assert!(host.local_worlds[0].is_none());
    assert!(host.runtime.model_assets.is_none());
    assert!(!host.runtime.quit);
}

#[path = "q3_downloads.rs"]
mod downloads;
