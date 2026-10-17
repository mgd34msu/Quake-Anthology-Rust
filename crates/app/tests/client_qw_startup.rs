#[path = "support/client_asset_frames.rs"]
mod asset_frames;
#[path = "support/frame_source.rs"]
mod frame_source;
#[path = "support/sound_registration.rs"]
#[allow(dead_code)]
mod sound_support;
#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod support;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult, FrameSource},
    model_assets::{ModelAssets, ModelRequest},
    snapshots::NativePlayerBinding,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{loopback::Endpoint, primitives::*, sys_events::*};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{connection::Commands, packet::Protocol},
    downloads::{Chunk, write_chunk},
    ingress::{Connection, ConnectionPhase, Incoming, Route},
    message::{Encoding, Writer},
};
use qa_render::{Assets, Limits};
use qa_session::timing::TickRate;
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    ops::ControlFlow,
    time::Duration,
};

const PROTOCOL: Protocol = Protocol::QuakeWorld28;
const RULES: RuleSetId = RuleSetId::QuakeWorld;
const SOCKET: u16 = 23;

struct Server {
    client: ClientId,
    address: SocketAddr,
    channel: Channel,
    commands: Commands,
    received: Vec<Vec<u8>>,
    time: EventTime,
}
impl Server {
    fn new(client: ClientId) -> Self {
        let mut channel = Channel::load(PROTOCOL.channel(), Endpoint::Server, 8192, 16).unwrap();
        channel.configure_snapshots(PROTOCOL).unwrap();
        channel.set_qport(28000 + client.0 as u16);
        Self {
            client,
            address: SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                27000 + client.0 as u16,
            )),
            channel,
            commands: Commands::load(PROTOCOL, 8192),
            received: Vec::new(),
            time: EventTime::default(),
        }
    }
    fn receive(&mut self, bytes: &[u8]) {
        let received = self.channel.receive(bytes, self.time).unwrap();
        if let Delivery::Payload(payload) = received.delivery {
            let length = self.commands.stage(payload).unwrap();
            self.commands
                .decode(
                    length,
                    received.header.sequence,
                    self.time.milliseconds() as i32,
                    &mut self.channel,
                    None,
                    |incoming| {
                        if let Incoming::CommandString { text, .. } = incoming {
                            self.received.push(text.to_vec());
                        }
                        ControlFlow::Continue(())
                    },
                )
                .unwrap();
        }
    }
    fn flush(&mut self, queue: &mut SysEventQueue) {
        while self.channel.prepare_output(self.time).unwrap().is_some() {
            assert!(
                self.channel
                    .submit_with(self.time, |bytes| queue
                        .push(SysEvent {
                            time: self.time,
                            kind: EventKind::Packet {
                                socket: SOCKET,
                                from: Peer::Socket(self.address),
                                bytes
                            },
                        })
                        .is_ok())
                    .unwrap()
                    .is_some()
            );
        }
    }
    fn count(&self, text: &[u8]) -> usize {
        self.received
            .iter()
            .filter(|command| command.as_slice() == text)
            .count()
    }
    fn prefixed(&self, prefix: &[u8]) -> Vec<&[u8]> {
        self.received
            .iter()
            .map(Vec::as_slice)
            .filter(|text| text.starts_with(prefix))
            .collect()
    }
}

struct Source {
    backend: asset_frames::Source,
    servers: Vec<Server>,
    time: u64,
    music: Vec<MusicControl>,
}
impl FrameSource for Source {
    fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
        self.backend.asset_preparer()
    }
    fn begin_frame(&mut self) -> Duration {
        self.time += 100;
        Duration::from_millis(self.time)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        for server in &mut self.servers {
            server.time = EventTime::from_milliseconds(self.time);
            server.flush(queue);
        }
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(self.time),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped fixture")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn send_packet(&mut self, socket: u16, address: SocketAddr, bytes: &[u8]) -> bool {
        assert_eq!(socket, SOCKET);
        self.servers
            .iter_mut()
            .find(|server| server.address == address)
            .unwrap()
            .receive(bytes);
        true
    }
    fn music(
        &mut self,
        _: &qa_content::vfs::Vfs,
        control: MusicControl,
        _: Option<&[u8]>,
        _: Option<&[u8]>,
    ) -> bool {
        self.music.push(control);
        true
    }
    fn sound(&mut self, _: Option<&qa_audio::Bank>, _: SoundEvent) -> bool {
        true
    }
}

fn setup(files: &[(&str, Vec<u8>)], clients: usize) -> (support::Fixture, FrameHost, Source) {
    let mut fixture = support::Fixture::new(files);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", RULES, Some(palette)).profile();
    let (scene, _) =
        qa_app::renderer::SceneComposer::load(Limits::default(), &mut assets, vec![]).unwrap();
    let mut owner = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([
                Some(palette),
                Some(palette),
                None,
                None,
                None,
            ]),
        )
        .unwrap();
    let mut runtime = Runtime::load(clients, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    runtime.model_assets = Some(owner);
    runtime.scene = Some(scene);
    runtime.sound_assets = Some(Box::new(
        qa_app::audio::SoundAssets::load(32, 4096).unwrap(),
    ));
    let servers: Vec<_> = (0..clients)
        .map(|index| Server::new(ClientId(index as u32)))
        .collect();
    for server in &servers {
        let mut channel = Channel::load(PROTOCOL.channel(), Endpoint::Client, 8192, 16).unwrap();
        channel.set_qport(28000 + server.client.0 as u16);
        let client = runtime
            .connect_client(
                SeatId::new(server.client.0 as u8).unwrap(),
                ClientPolicy {
                    input: RULES,
                    client: RULES,
                    movement: RuleSetId::Quake3,
                    trace: RuleSetId::Quake2,
                },
                NativePlayerBinding::Supplied(7),
                Connection {
                    phase: ConnectionPhase::Connected,
                    route: Route {
                        socket: SOCKET,
                        peer: Peer::Socket(server.address),
                    },
                    channel,
                    output: None,
                    pending_output: None,
                    commands: Some(Commands::load(PROTOCOL, 8192)),
                },
                None,
            )
            .unwrap();
        assert_eq!(client, server.client);
        assert!(!runtime.server.clients[client.0 as usize].server_authoritative());
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    for index in 0..clients {
        host.local_clients[index] = Some(ClientId(index as u32));
    }
    host.bind_client_model_assets(
        (0..clients).map(|index| (ClientId(index as u32), profile, None)),
    );
    let backend = asset_frames::Source::load(host.runtime.model_assets.as_ref().unwrap().assets());
    host.publish_model_admission(backend.cpu.admitted_assets().unwrap())
        .unwrap();
    host.runtime
        .model_assets
        .as_mut()
        .unwrap()
        .end_initial_registration();
    (
        fixture,
        host,
        Source {
            backend,
            servers,
            time: 100,
            music: vec![],
        },
    )
}

fn step(host: &mut FrameHost, source: &mut Source) {
    assert_eq!(host.frame(source, true).drains, 2);
}
fn settle(host: &mut FrameHost, source: &mut Source) {
    for _ in 0..4 {
        step(host, source);
    }
}
fn send(host: &mut FrameHost, source: &mut Source, client: usize, bytes: &[u8]) {
    let time = EventTime::from_milliseconds(source.time + 1);
    let server = &mut source.servers[client];
    server.time = time;
    prime(host, server, time);
    server.channel.queue_reliable(bytes).unwrap();
    server.flush(&mut host.queue);
    step(host, source);
}
fn prime(host: &mut FrameHost, server: &mut Server, time: EventTime) {
    let mut outgoing = frame_source::Sink::default();
    host.runtime
        .send_client_command(server.client, &UserCmd::default(), time, &mut outgoing);
    assert!(
        !outgoing.packets.is_empty(),
        "native CLIENT control packet must reach its server"
    );
    for packet in outgoing.packets {
        assert!(packet.admitted);
        server.receive(&packet.bytes);
    }
}
fn message(write: impl FnOnce(&mut Writer<'_>)) -> Vec<u8> {
    let mut bytes = [0; 8192];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    write(&mut writer);
    writer.bytes().to_vec()
}
fn stuff(writer: &mut Writer<'_>, text: &[u8]) {
    writer.write_bits(9, 8).unwrap();
    writer.write_string(PROTOCOL.string_policy(), text).unwrap();
}
fn serverdata(server: i32, slot: u8, extra: bool) -> Vec<u8> {
    message(|writer| {
        writer.write_bits(11, 8).unwrap();
        writer.write_bits(28, 32).unwrap();
        writer.write_bits(server as u32, 32).unwrap();
        writer
            .write_string(PROTOCOL.string_policy(), b"id1")
            .unwrap();
        writer.write_bits(u32::from(slot), 8).unwrap();
        writer
            .write_string(PROTOCOL.string_policy(), b"Native startup fixture")
            .unwrap();
        for number in [800.0, 100.0, 320.0, 500.0, 10.0, 0.7, 10.0, 4.0, 4.0, 1.0] {
            writer.write_float(number).unwrap();
        }
        if extra {
            writer.write_bits(32, 8).unwrap();
            writer.write_bits(3, 8).unwrap();
            stuff(
                writer,
                b"fullserverinfo \"\\hostname\\native fixture\\*gamedir\\id1\"\n",
            );
        }
    })
}
fn list(sound: bool, first: u8, names: &[&[u8]], next: u8) -> Vec<u8> {
    message(|writer| {
        writer.write_bits(if sound { 46 } else { 45 }, 8).unwrap();
        writer.write_bits(u32::from(first), 8).unwrap();
        for name in names {
            writer.write_string(PROTOCOL.string_policy(), name).unwrap();
        }
        writer.write_string(PROTOCOL.string_policy(), b"").unwrap();
        writer.write_bits(u32::from(next), 8).unwrap();
    })
}
fn prespawn(host: &FrameHost, server: i32) -> Vec<u8> {
    let checksum = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .catalog()
        .world_native_checksums(b"maps/ready.bsp")
        .and_then(|checksums| checksums.quakeworld_lumps)
        .unwrap();
    format!("prespawn {server} 0 {}", checksum as i32).into_bytes()
}
fn ordinary_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("maps/ready.bsp", support::world_map(16.0)),
        ("progs/test.mdl", support::mdl()),
        ("sound/first.wav", sound_support::wav()),
        ("sound/second.wav", sound_support::wav()),
    ]
}

#[test]
fn continued_lists_admit_actual_media_before_prespawn_and_structured_signon() {
    let files = ordinary_files();
    let checksum = qa_formats::bsp::Bsp::parse(&files[0].1)
        .unwrap()
        .quakeworld_checksum2()
        .unwrap();
    let (_fixture, mut host, mut source) = setup(&files, 1);
    source.backend.enabled = false;
    send(&mut host, &mut source, 0, &serverdata(137, 5, true));
    settle(&mut host, &mut source);
    assert_eq!(
        source.servers[0].prefixed(b"soundlist "),
        [b"soundlist 137 0".as_slice()]
    );
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        Some(5)
    );
    assert!(!host.runtime.snapshots[0].as_ref().unwrap().spectator());
    let resources = host.runtime.client_resources[0].as_ref().unwrap();
    assert_eq!(resources.received_game_directory(), Some(b"id1".as_slice()));
    assert_eq!(
        resources.received_server_info(),
        Some(b"\\hostname\\native fixture\\*gamedir\\id1".as_slice())
    );
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .server_info()
            .unwrap()
            .level_name
            .as_bytes(),
        b"Native startup fixture"
    );
    assert_eq!(
        source
            .music
            .iter()
            .filter(|control| matches!(
                control,
                MusicControl::Track {
                    number: 3,
                    looping: true,
                    ..
                }
            ))
            .count(),
        1,
        "CD track in the same packet must still reach presentation"
    );

    send(
        &mut host,
        &mut source,
        0,
        &list(true, 0, &[b"first.wav"], 1),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"soundlist 137 1"), 1);
    assert!(source.servers[0].prefixed(b"modellist ").is_empty());
    send(
        &mut host,
        &mut source,
        0,
        &list(true, 1, &[b"second.wav"], 0),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"modellist 137 0"), 1);
    for path in ["sound/first.wav", "sound/second.wav"] {
        let bank = host.runtime.sound_bank().unwrap();
        assert!(bank.sample(bank.find(path).unwrap()).is_some());
    }
    send(
        &mut host,
        &mut source,
        0,
        &list(false, 0, &[b"maps/ready.bsp"], 1),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"modellist 137 1"), 1);
    send(
        &mut host,
        &mut source,
        0,
        &list(false, 1, &[b"progs/test.mdl", b"*1"], 0),
    );
    settle(&mut host, &mut source);
    assert!(source.servers[0].prefixed(b"prespawn ").is_empty());
    assert!(host.local_worlds[0].is_none());
    source.backend.enabled = true;
    settle(&mut host, &mut source);
    let expected = format!("prespawn 137 0 {}", checksum as i32);
    assert_eq!(
        source.servers[0].prefixed(b"prespawn "),
        [expected.as_bytes()]
    );
    assert_eq!(prespawn(&host, 137), expected.as_bytes());
    assert!(host.local_worlds[0].is_some());
    assert!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .models
            .collision_world()
            .is_some()
    );
    assert_eq!(
        source.backend.cpu.admitted_assets(),
        Some(
            host.runtime
                .model_assets
                .as_ref()
                .unwrap()
                .assets()
                .counts()
        )
    );

    send(
        &mut host,
        &mut source,
        0,
        &message(|writer| stuff(writer, b"cmd prespawn 137 1\n")),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"prespawn 137 1"), 1);
    send(
        &mut host,
        &mut source,
        0,
        &message(|writer| stuff(writer, b"cmd spawn 137 0\n")),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"spawn 137 0"), 1);
    send(
        &mut host,
        &mut source,
        0,
        &message(|writer| stuff(writer, b"skins\n")),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"begin 137"), 1);
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"begin 137"), 1);
    let replies: Vec<_> = source.servers[0]
        .received
        .iter()
        .map(Vec::as_slice)
        .filter(|text| {
            [
                b"soundlist ".as_slice(),
                b"modellist ",
                b"prespawn ",
                b"spawn ",
                b"begin ",
            ]
            .iter()
            .any(|prefix| text.starts_with(prefix))
        })
        .collect();
    assert_eq!(
        replies,
        [
            b"soundlist 137 0".as_slice(),
            b"soundlist 137 1",
            b"modellist 137 0",
            b"modellist 137 1",
            expected.as_bytes(),
            b"prespawn 137 1",
            b"spawn 137 0",
            b"begin 137"
        ]
    );
    let history = host
        .runtime
        .network
        .get(ClientId(0), Endpoint::Client)
        .unwrap()
        .commands
        .as_ref()
        .unwrap()
        .history()
        .unwrap()
        .number();
    send(
        &mut host,
        &mut source,
        0,
        &message(|writer| stuff(writer, b"reconnect\n")),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"new"), 1);
    assert_eq!(
        host.runtime
            .network
            .get(ClientId(0), Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        history,
        "waiting for replacement serverdata must not publish movement commands"
    );
}

#[test]
fn missing_sound_download_finishes_before_models_are_requested() {
    let files = [("maps/ready.bsp", support::world_map(16.0))];
    let (_fixture, mut host, mut source) = setup(&files, 1);
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    host.configure_client_downloads(ClientId(0), mount, mount, 8192, 64)
        .unwrap();
    send(&mut host, &mut source, 0, &serverdata(19, 0, false));
    send(
        &mut host,
        &mut source,
        0,
        &list(true, 0, &[b"fetched.wav"], 0),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download sound/fetched.wav"), 1);
    assert!(source.servers[0].prefixed(b"modellist ").is_empty());
    let wav = sound_support::wav();
    for (percent, bytes) in [(50, &wav[..20]), (100, &wav[20..])] {
        let mut packet = [0; 128];
        let length = write_chunk(
            PROTOCOL,
            Chunk::Sequential {
                compressed: false,
                percent,
                bytes: Some(bytes),
            },
            &mut packet,
        )
        .unwrap();
        send(&mut host, &mut source, 0, &packet[..length]);
        settle(&mut host, &mut source);
        if percent == 50 {
            assert_eq!(source.servers[0].count(b"nextdl"), 1);
            assert!(host.runtime.vfs.open(b"sound/fetched.wav").is_none());
            assert!(source.servers[0].prefixed(b"modellist ").is_empty());
        }
    }
    assert!(host.runtime.vfs.open(b"sound/fetched.wav").is_some());
    assert!(
        host.runtime
            .sound_bank()
            .unwrap()
            .find("sound/fetched.wav")
            .is_some()
    );
    assert_eq!(source.servers[0].count(b"modellist 19 0"), 1);
    send(
        &mut host,
        &mut source,
        0,
        &list(false, 0, &[b"maps/ready.bsp"], 0),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(&prespawn(&host, 19)), 1);
}

#[test]
fn full_reliable_commands_retry_once_and_new_serverdata_resets_the_native_assignment() {
    let (_fixture, mut host, mut source) = setup(&ordinary_files(), 1);
    send(&mut host, &mut source, 0, &serverdata(-17, 128 | 31, true));
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"soundlist -17 0"), 1);
    let time = EventTime::from_milliseconds(source.time + 1);
    source.servers[0].time = time;
    prime(&mut host, &mut source.servers[0], time);
    let channel = &mut host
        .runtime
        .network
        .get_mut(ClientId(0), Endpoint::Client)
        .unwrap()
        .channel;
    let mut pressure = 0;
    while channel.queue_client_command(PROTOCOL, b"pressure").is_ok() {
        pressure += 1;
        assert!(pressure < 1024);
    }
    assert!(pressure > 0);
    let rejected = channel.counts().client_command_rejected;
    source.servers[0]
        .channel
        .queue_reliable(&list(true, 0, &[b"first.wav"], 1))
        .unwrap();
    source.servers[0].flush(&mut host.queue);
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut FrameResult::default());
    assert!(
        host.runtime
            .network
            .get(ClientId(0), Endpoint::Client)
            .unwrap()
            .channel
            .counts()
            .client_command_rejected
            > rejected,
        "the startup reply must encounter the full native reliable queue"
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"pressure"), pressure);
    assert_eq!(source.servers[0].count(b"soundlist -17 0"), 1);
    assert_eq!(source.servers[0].count(b"soundlist -17 1"), 1);
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        Some(31)
    );
    assert!(host.runtime.snapshots[0].as_ref().unwrap().spectator());
    assert_eq!(
        host.runtime.prediction[0].player.movement.mode,
        MovementMode::Spectator
    );
    send(&mut host, &mut source, 0, &list(true, 1, &[], 0));
    send(
        &mut host,
        &mut source,
        0,
        &list(false, 0, &[b"maps/ready.bsp"], 0),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(&prespawn(&host, -17)), 1);
    send(
        &mut host,
        &mut source,
        0,
        &message(|writer| stuff(writer, b"reconnect\n")),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"new"), 1);
    send(&mut host, &mut source, 0, &serverdata(23, 2, false));
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"soundlist 23 0"), 1);
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        Some(2)
    );
    assert!(!host.runtime.snapshots[0].as_ref().unwrap().spectator());
    send(&mut host, &mut source, 0, &list(true, 0, &[], 0));
    send(
        &mut host,
        &mut source,
        0,
        &list(false, 0, &[b"maps/ready.bsp"], 0),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(&prespawn(&host, 23)), 1);
}

#[test]
fn malformed_required_model_closes_only_its_client_and_healthy_media_still_admits() {
    let mut files = ordinary_files();
    files.push(("progs/broken.mdl", b"not a model".to_vec()));
    let (_fixture, mut host, mut source) = setup(&files, 2);
    for client in 0..2 {
        send(
            &mut host,
            &mut source,
            client,
            &serverdata(41 + client as i32, client as u8, false),
        );
        send(&mut host, &mut source, client, &list(true, 0, &[], 0));
    }
    send(
        &mut host,
        &mut source,
        0,
        &list(false, 0, &[b"maps/ready.bsp", b"progs/broken.mdl"], 0),
    );
    send(
        &mut host,
        &mut source,
        1,
        &list(false, 0, &[b"maps/ready.bsp", b"progs/test.mdl"], 0),
    );
    settle(&mut host, &mut source);
    assert!(source.servers[0].prefixed(b"prespawn ").is_empty());
    assert!(
        host.runtime.snapshots[0]
            .as_ref()
            .is_none_or(|binding| binding.presentation_closed())
    );
    assert_eq!(source.servers[1].count(&prespawn(&host, 42)), 1);
    assert!(
        !host.runtime.snapshots[1]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert!(host.local_worlds[1].is_some());
}
