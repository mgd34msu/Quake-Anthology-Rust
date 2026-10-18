#[path = "support/client_asset_frames.rs"]
mod asset_frames;
#[path = "client_q2_startup/published_pictures.rs"]
mod published_pictures;
#[path = "support/q2_startup.rs"]
mod q2_files;
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
use qa_core::{configstrings::ConfigLayout, loopback::Endpoint, primitives::*, sys_events::*};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{connection::Commands, packet::Protocol},
    downloads::{Chunk, write_chunk},
    ingress::{Connection, ConnectionPhase, Incoming, Route},
    message::{Encoding, Writer},
    snapshots::{self, Frame as Snapshot, Q2EntityPolicy, Q2ReproRing, Q2Ring},
    states,
};
use qa_render::{Assets, Limits};
use qa_session::timing::TickRate;
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    ops::ControlFlow,
    time::Duration,
};

const SOCKET: u16 = 24;
const CLASSIC: Protocol = Protocol::Quake2_34;
const RERELEASE: Protocol = Protocol::Quake2Repro1038;

struct Server {
    client: ClientId,
    address: SocketAddr,
    protocol: Protocol,
    channel: Channel,
    commands: Commands,
    received: Vec<Vec<u8>>,
    time: EventTime,
}
impl Server {
    fn new(client: ClientId, protocol: Protocol) -> Self {
        let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
        channel.configure_snapshots(protocol).unwrap();
        channel.set_qport(29000 + client.0 as u16);
        Self {
            client,
            address: SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                27100 + client.0 as u16,
            )),
            protocol,
            channel,
            commands: Commands::load(protocol, 8192),
            received: vec![],
            time: EventTime::default(),
        }
    }
    fn connect(&self, runtime: &mut Runtime) -> ClientId {
        let mut channel =
            Channel::load(self.protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
        channel.set_qport(29000 + self.client.0 as u16);
        let client = runtime
            .connect_client(
                SeatId::new(self.client.0 as u8).unwrap(),
                ClientPolicy {
                    input: self.protocol.rule_set(),
                    client: self.protocol.rule_set(),
                    movement: RuleSetId::Quake3,
                    trace: RuleSetId::Quake,
                },
                NativePlayerBinding::Supplied(7),
                Connection {
                    phase: ConnectionPhase::Connected,
                    route: Route {
                        socket: SOCKET,
                        peer: Peer::Socket(self.address),
                    },
                    channel,
                    output: None,
                    pending_output: None,
                    commands: Some(Commands::load(self.protocol, 8192)),
                },
                None,
            )
            .unwrap();
        assert_eq!(client, self.client);
        assert!(!runtime.server.clients[client.0 as usize].server_authoritative());
        client
    }
    fn receive(&mut self, bytes: &[u8]) {
        let received = self.channel.receive(bytes, self.time).unwrap();
        if let Delivery::Payload(payload) = received.delivery
            && !payload.is_empty()
        {
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
        loop {
            if self.channel.prepare_output(self.time).unwrap().is_none() {
                break;
            }
            assert!(
                self.channel
                    .submit_with(self.time, |bytes| queue
                        .push(SysEvent {
                            time: self.time,
                            kind: EventKind::Packet {
                                socket: SOCKET,
                                from: Peer::Socket(self.address),
                                bytes,
                            },
                        })
                        .is_ok())
                    .unwrap()
                    .is_some()
            );
            // Q2 always permits an empty maintenance datagram. Only drain
            // further packets when the same native message has fragments.
            if !self.channel.pending_fragments() {
                break;
            }
        }
    }
    fn count(&self, text: &[u8]) -> usize {
        self.received
            .iter()
            .filter(|value| value.as_slice() == text)
            .count()
    }
    fn prefixed(&self, prefix: &[u8]) -> Vec<&[u8]> {
        self.received
            .iter()
            .map(Vec::as_slice)
            .filter(|value| value.starts_with(prefix))
            .collect()
    }
}

struct Source {
    backend: asset_frames::Source,
    reject_assets: bool,
    hud_target: bool,
    servers: Vec<Server>,
    time: u64,
}
impl qa_app::renderer::AssetPreparer for Source {
    fn target_size(&self) -> Option<[u32; 2]> {
        self.hud_target.then_some([64, 48])
    }
    fn prepare_assets(&mut self, assets: &Assets, required: Option<Limits>) -> Result<(), String> {
        if self.reject_assets {
            self.backend.attempts += 1;
            return Err("published model material rejected".to_owned());
        }
        qa_app::renderer::AssetPreparer::prepare_assets(&mut self.backend, assets, required)
    }
}
impl FrameSource for Source {
    fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
        if self.backend.enabled {
            Some(self)
        } else {
            None
        }
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
    fn sound(&mut self, _: Option<&qa_audio::Bank>, _: SoundEvent) -> bool {
        true
    }
}

fn setup(
    files: &[(&str, Vec<u8>)],
    protocols: &[Protocol],
) -> (support::Fixture, FrameHost, Source) {
    let mut fixture = support::Fixture::new(files);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profiles: Vec<_> = protocols
        .iter()
        .map(|protocol| ModelRequest::native("", protocol.rule_set(), Some(palette)).profile())
        .collect();
    let (scene, _) =
        qa_app::renderer::SceneComposer::load(Limits::default(), &mut assets, vec![]).unwrap();
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        assets,
        profiles.iter().copied(),
        [],
        Default::default(),
    )
    .unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered(
                &fixture.vfs,
                [
                    Some(palette),
                    Some(palette),
                    Some(palette),
                    Some(palette),
                    None,
                ],
            ),
        )
        .unwrap();
    let mut runtime = Runtime::load(protocols.len(), []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    runtime.model_assets = Some(owner);
    runtime.scene = Some(scene);
    runtime.sound_assets = Some(Box::new(
        qa_app::audio::SoundAssets::load(&runtime.vfs, 32, 4096).unwrap(),
    ));
    let servers: Vec<_> = protocols
        .iter()
        .enumerate()
        .map(|(index, &protocol)| Server::new(ClientId(index as u32), protocol))
        .collect();
    for server in &servers {
        server.connect(&mut runtime);
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    for index in 0..protocols.len() {
        host.local_clients[index] = Some(ClientId(index as u32));
    }
    host.bind_client_model_assets(
        profiles
            .into_iter()
            .enumerate()
            .map(|(index, profile)| (ClientId(index as u32), profile, None)),
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
            reject_assets: false,
            hud_target: false,
            servers,
            time: 100,
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
    let server = &mut source.servers[client];
    server.time = EventTime::from_milliseconds(source.time + 1);
    server.channel.queue_reliable(bytes).unwrap();
    server.flush(&mut host.queue);
    step(host, source);
}
fn message(write: impl FnOnce(&mut Writer<'_>)) -> Vec<u8> {
    let mut bytes = [0; 8192];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    write(&mut writer);
    writer.bytes().to_vec()
}
fn stuff(protocol: Protocol, text: &[u8]) -> Vec<u8> {
    message(|writer| {
        writer.write_bits(11, 8).unwrap();
        writer.write_string(protocol.string_policy(), text).unwrap();
    })
}
fn serverdata(
    protocol: Protocol,
    count: i32,
    slot: i16,
    attract: bool,
    flags: u16,
    fps: u8,
) -> Vec<u8> {
    message(|writer| {
        writer.write_bits(12, 8).unwrap();
        writer.write_bits(protocol.number(), 32).unwrap();
        writer.write_bits(count as u32, 32).unwrap();
        writer.write_bits(u32::from(attract), 8).unwrap();
        writer
            .write_string(protocol.string_policy(), b"baseq2")
            .unwrap();
        writer.write_bits(u32::from(slot as u16), 16).unwrap();
        writer
            .write_string(protocol.string_policy(), b"Native Q2 startup")
            .unwrap();
        if protocol == RERELEASE {
            writer.write_bits(1024, 16).unwrap();
            writer.write_bits(2, 8).unwrap();
            writer.write_bits(u32::from(flags), 16).unwrap();
            writer.write_bits(u32::from(fps), 8).unwrap();
        }
    })
}
fn layout(host: &FrameHost, client: usize) -> ConfigLayout {
    host.runtime.client_resources[client]
        .as_ref()
        .unwrap()
        .state
        .layout
}
fn config(protocol: Protocol, row: usize, text: &[u8]) -> Vec<u8> {
    asset_frames::config(protocol, row, text)
}
fn baseline(protocol: Protocol, stream: bool, number: u16) -> Vec<u8> {
    message(|writer| {
        if !stream {
            writer.write_bits(14, 8).unwrap();
        }
        if protocol == CLASSIC {
            let mut words = [0; states::Q2_ENTITY_WORDS];
            words[0] = 2;
            words[8] = 32f32.to_bits();
            states::write_q2_entity(
                writer,
                u32::from(number),
                &[0; states::Q2_ENTITY_WORDS],
                Some(&words),
                true,
                true,
            )
            .unwrap();
        } else {
            let mut words = [0; states::Q2_RERELEASE_ENTITY_WORDS];
            words[0] = 2;
            words[8] = 32f32.to_bits();
            states::write_q2_repro_entity(
                writer,
                number,
                &[0; states::Q2_RERELEASE_ENTITY_WORDS],
                Some(&words),
                true,
            )
            .unwrap();
        }
    })
}
fn ordinary_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("maps/ready.bsp", q2_files::world(b"wall")),
        ("textures/wall.wal", q2_files::wal(11)),
        ("models/test/tris.md2", support::md2()),
        ("models/test/skin0.pcx", support::pcx(11)),
        ("models/test/skin1.pcx", support::pcx(22)),
        ("sound/step.wav", sound_support::wav()),
    ]
}
fn world_rows(protocol: Protocol, layout: ConfigLayout, model: bool) -> Vec<u8> {
    let checksum = qa_core::checksum::block_checksum(&q2_files::world(b"wall"));
    let mut bytes = config(
        protocol,
        layout.resources[0].get(1).unwrap(),
        b"maps/ready.bsp",
    );
    bytes.extend(config(
        protocol,
        layout.map_checksum().unwrap(),
        format!("{}", checksum as i32).as_bytes(),
    ));
    if model {
        bytes.extend(config(
            protocol,
            layout.resources[0].get(2).unwrap(),
            b"models/test/tris.md2",
        ));
        bytes.extend(config(
            protocol,
            layout.resources[1].get(1).unwrap(),
            b"step.wav",
        ));
    }
    bytes
}
fn history(host: &FrameHost, client: usize) -> u64 {
    host.runtime
        .network
        .get(ClientId(client as u32), Endpoint::Client)
        .unwrap()
        .commands
        .as_ref()
        .unwrap()
        .history()
        .unwrap()
        .number()
}
fn assert_admitted(
    host: &FrameHost,
    source: &Source,
    client: usize,
    model: bool,
    world: (&[u8], &[u8]),
) {
    assert!(host.local_worlds[client].is_some());
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let resources = &host.runtime.client_resources[client]
        .as_ref()
        .unwrap()
        .state;
    assert!(resources.models.collision_world().is_some());
    assert_eq!(
        owner
            .catalog()
            .world_native_checksums(world.0)
            .map(|checksums| checksums.whole_file),
        Some(qa_core::checksum::block_checksum(world.1))
    );
    if model {
        assert!(
            resources
                .models
                .resolve(
                    ModelRef::native(ResourceNamespace::Client(ClientId(client as u32)), 2)
                        .unwrap(),
                    0,
                    owner.catalog(),
                    Default::default()
                )
                .is_some()
        );
        let bank = host.runtime.sound_bank().unwrap();
        assert!(bank.sample(bank.find("sound/step.wav").unwrap()).is_some());
    }
    assert_eq!(
        source.backend.cpu.admitted_assets(),
        Some(owner.assets().counts())
    );
}

#[test]
fn classic_paginated_signon_retries_full_reply_and_admits_media_before_begin() {
    let (_fixture, mut host, mut source) = setup(&ordinary_files(), &[CLASSIC]);
    source.backend.enabled = false;
    let mut header = serverdata(CLASSIC, -77, 255, false, 0, 0);
    header.extend(stuff(CLASSIC, b"cmd configstrings -77 0\n"));
    send(&mut host, &mut source, 0, &header);
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"configstrings -77 0"), 1);
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        Some(255)
    );
    assert_eq!(history(&host, 0), 0);
    let data = host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .received_q2_server_data()
        .unwrap();
    assert_eq!(
        (
            data.server_count,
            data.client_number,
            data.game_directory,
            data.level_name
        ),
        (
            -77,
            255,
            b"baseq2".as_slice(),
            b"Native Q2 startup".as_slice()
        )
    );
    assert!(data.rerelease.is_none());

    let channel = &mut host
        .runtime
        .network
        .get_mut(ClientId(0), Endpoint::Client)
        .unwrap()
        .channel;
    channel.queue_client_command(CLASSIC, b"pressure").unwrap();
    // Protocol 34 executes at most seven strings in one packet. Fill its
    // reliable ring with native clc_nop records, retaining one visible string.
    let mut pressure = 0;
    while channel.queue_reliable(&[1]).is_ok() {
        pressure += 1;
        assert!(pressure < 1024);
    }
    assert!(pressure > 0);
    let rejected = channel.counts().client_command_rejected;
    let mut page = world_rows(CLASSIC, layout(&host, 0), true);
    page.extend(stuff(CLASSIC, b"cmd configstrings -77 288\n"));
    source.servers[0].channel.queue_reliable(&page).unwrap();
    source.servers[0].time = EventTime::from_milliseconds(source.time + 1);
    source.servers[0].flush(&mut host.queue);
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
        "continued configstrings must encounter native reliable Full"
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"pressure"), 1);
    assert_eq!(source.servers[0].count(b"configstrings -77 288"), 1);
    send(
        &mut host,
        &mut source,
        0,
        &stuff(CLASSIC, b"cmd baselines -77 0\n"),
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"baselines -77 0"), 1);
    let mut page = baseline(CLASSIC, false, 7);
    page.extend(stuff(CLASSIC, b"cmd baselines -77 8\n"));
    send(&mut host, &mut source, 0, &page);
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"baselines -77 8"), 1);
    send(
        &mut host,
        &mut source,
        0,
        &stuff(CLASSIC, b"precache -78\n"),
    );
    assert!(source.servers[0].prefixed(b"begin ").is_empty());
    send(
        &mut host,
        &mut source,
        0,
        &stuff(CLASSIC, b"precache -77\n"),
    );
    settle(&mut host, &mut source);
    assert!(source.servers[0].prefixed(b"begin ").is_empty());
    assert!(host.local_worlds[0].is_none());
    assert_eq!(history(&host, 0), 0);
    source.backend.enabled = true;
    settle(&mut host, &mut source);
    assert_admitted(
        &host,
        &source,
        0,
        true,
        (b"maps/ready.bsp", &q2_files::world(b"wall")),
    );
    assert_eq!(
        source.servers[0].prefixed(b"begin "),
        [b"begin -77".as_slice()]
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"begin -77"), 1);
}

fn rerelease_stream(
    layout: ConfigLayout,
    separate: bool,
    count: i32,
    world: (&[u8], &[u8]),
) -> Vec<u8> {
    let checksum = format!("{}", qa_core::checksum::block_checksum(world.1) as i32);
    let mut bytes = raw_configs(
        layout,
        !separate,
        &[
            (layout.resources[0].get(1).unwrap(), world.0),
            (layout.resources[0].get(2).unwrap(), b"models/test/tris.md2"),
            (layout.resources[1].get(1).unwrap(), b"step.wav"),
            (layout.map_checksum().unwrap(), checksum.as_bytes()),
        ],
    );
    if separate {
        bytes.push(39);
    }
    bytes.extend(baseline(RERELEASE, true, 7));
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(stuff(RERELEASE, format!("precache {count}\n").as_bytes()));
    bytes
}
fn raw_configs(layout: ConfigLayout, combined: bool, rows: &[(usize, &[u8])]) -> Vec<u8> {
    message(|writer| {
        writer
            .write_bits(if combined { 36 } else { 38 }, 8)
            .unwrap();
        for &(index, text) in rows {
            writer.write_bits(index as u32, 16).unwrap();
            writer
                .write_string(RERELEASE.string_policy(), text)
                .unwrap();
        }
        writer.write_bits(layout.count as u32, 16).unwrap();
    })
}
fn raw_baselines(number: u16) -> Vec<u8> {
    let mut bytes = vec![39];
    bytes.extend(baseline(RERELEASE, true, number));
    bytes.extend(0u16.to_le_bytes());
    bytes
}

fn snapshot(protocol: Protocol, sequence: u32) -> Vec<u8> {
    message(|writer| {
        if protocol == CLASSIC {
            let mut ring = Q2Ring::load(8, 1024, 32, None).unwrap();
            ring.store(Snapshot {
                sequence,
                time: ThinkTime::Milliseconds(i64::from(sequence) * 100),
                command: 0,
                flags: 0,
                qw_peers: 0,
                qw_dead: 0,
                areas: &[],
                player: &[0; states::Q2_PLAYER_WORDS],
                entities: &[],
            })
            .unwrap();
            snapshots::write_q2(writer, &ring, sequence, None, 0).unwrap();
        } else {
            let mut ring = Q2ReproRing::load(8, 8192, 255, None).unwrap();
            ring.store(Snapshot {
                sequence,
                time: ThinkTime::Milliseconds(i64::from(sequence) * 25),
                command: 0,
                flags: 0,
                qw_peers: 0,
                qw_dead: 0,
                areas: &[],
                player: &[0; states::Q2_REPRO_PLAYER_WORDS],
                entities: &[],
            })
            .unwrap();
            snapshots::write_q2_repro(writer, &mut ring, sequence, None, Q2EntityPolicy::default())
                .unwrap();
        }
    })
}

#[test]
fn rerelease_raw_streams_preserve_supplied_layout_fps_and_new_generation() {
    let (_fixture, mut host, mut source) = setup(&ordinary_files(), &[RERELEASE]);
    for (index, (flags, expected_count, attract, period)) in [
        (0, 12448, false, 20),
        (0x8000, 2080, true, 16),
        (0x8010, 13630, false, 20),
    ]
    .into_iter()
    .enumerate()
    {
        let generation = -300 + index as i32;
        let before_history = history(&host, 0);
        send(
            &mut host,
            &mut source,
            0,
            &serverdata(RERELEASE, generation, 123, attract, flags, 59),
        );
        let resources = host.runtime.client_resources[0].as_ref().unwrap();
        let data = resources.received_q2_server_data().unwrap();
        assert_eq!(data.server_count, generation);
        assert_eq!(data.client_number, 123);
        let metadata = data.rerelease.unwrap();
        assert_eq!(
            (
                metadata.revision,
                metadata.server_state,
                metadata.flags,
                metadata.server_fps
            ),
            (1024, 2, flags, 59)
        );
        assert_eq!(data.frame_milliseconds(), period);
        assert_eq!(resources.state.layout.count, expected_count);
        assert_eq!(resources.state.layout, data.resource_layout());
        assert_eq!(history(&host, 0), before_history);
        assert_eq!(
            host.runtime.snapshots[0].as_ref().unwrap().native_client(),
            Some(123)
        );
        assert!(source.servers[0].prefixed(b"configstrings ").is_empty());
        let resources = layout(&host, 0);
        if flags & 0x8000 != 0 {
            // Protocol 1038 retains 96-byte config rows even when its game
            // namespace is classic or extended game-3 compatibility.
            let last_statusbar = resources.map_checksum().unwrap() - 3;
            let last_general = resources.count - 1;
            let ordinary = [b'o'; 95];
            let statusbar = [b' '; 95];
            let general = [b'g'; 95];
            let mut rows = config(RERELEASE, 0, &ordinary);
            rows.extend(config(RERELEASE, last_statusbar + 1, b"0"));
            rows.extend(config(RERELEASE, last_statusbar, &statusbar));
            rows.extend(config(RERELEASE, last_general - 1, b"left"));
            rows.extend(config(RERELEASE, last_general, &general));
            send(&mut host, &mut source, 0, &rows);
            let configs = &host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .configs;
            for (row, expected) in [
                (0, ordinary.as_slice()),
                (last_statusbar, statusbar.as_slice()),
                (last_general, general.as_slice()),
                (last_statusbar + 1, b"0"),
                (last_general - 1, b"left"),
            ] {
                assert_eq!(
                    configs.config(row).unwrap().text,
                    expected,
                    "RR flags {flags:#x} config row {row}"
                );
            }
            assert_eq!(
                source.servers[0].count(format!("begin {generation}").as_bytes()),
                0
            );
        }
        match index {
            0 => {
                let checksum = format!(
                    "{}",
                    qa_core::checksum::block_checksum(&q2_files::world(b"wall")) as i32
                );
                let attempts = source.backend.attempts;
                for bytes in [
                    raw_configs(
                        resources,
                        false,
                        &[
                            (resources.resources[0].get(1).unwrap(), b"maps/ready.bsp"),
                            (resources.map_checksum().unwrap(), checksum.as_bytes()),
                        ],
                    ),
                    raw_configs(
                        resources,
                        false,
                        &[
                            (
                                resources.resources[0].get(2).unwrap(),
                                b"models/test/tris.md2",
                            ),
                            (resources.resources[1].get(1).unwrap(), b"step.wav"),
                        ],
                    ),
                    raw_baselines(7),
                    raw_baselines(8),
                ] {
                    send(&mut host, &mut source, 0, &bytes);
                    settle(&mut host, &mut source);
                    assert_eq!(
                        source.backend.attempts, attempts,
                        "native chunk terminators do not complete precache"
                    );
                    assert_eq!(
                        source.servers[0].count(format!("begin {generation}").as_bytes()),
                        0
                    );
                    assert_eq!(history(&host, 0), before_history);
                }
                send(
                    &mut host,
                    &mut source,
                    0,
                    &stuff(RERELEASE, format!("precache {generation}\n").as_bytes()),
                );
            }
            1 => {
                // An empty gamestate emits no stream markers. Ordinary
                // config updates still provide this fixture's required map.
                let mut rows = world_rows(RERELEASE, resources, true);
                rows.extend(stuff(
                    RERELEASE,
                    format!("precache {generation}\n").as_bytes(),
                ));
                send(&mut host, &mut source, 0, &rows);
            }
            _ => {
                let stream = rerelease_stream(
                    resources,
                    false,
                    generation,
                    (b"maps/ready.bsp", &q2_files::world(b"wall")),
                );
                send(&mut host, &mut source, 0, &stream);
            }
        }
        settle(&mut host, &mut source);
        assert_admitted(
            &host,
            &source,
            0,
            true,
            (b"maps/ready.bsp", &q2_files::world(b"wall")),
        );
        assert_eq!(
            source.servers[0].count(format!("begin {generation}").as_bytes()),
            1
        );
        assert_eq!(
            history(&host, 0),
            before_history,
            "begin is not an authoritative snapshot"
        );
        send(&mut host, &mut source, 0, &snapshot(RERELEASE, 1));
        let current = host
            .runtime
            .network
            .get(ClientId(0), Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        assert_eq!(current.time(), ThinkTime::Milliseconds(i64::from(period)));
        settle(&mut host, &mut source);
        assert!(history(&host, 0) > before_history);
        assert_eq!(
            source.servers[0].count(format!("begin {generation}").as_bytes()),
            1
        );
    }
}

#[test]
fn malformed_headers_drop_only_their_client_and_a_fresh_channel_can_sign_on() {
    for protocol in [CLASSIC, RERELEASE] {
        for truncate in [true, false] {
            let (_fixture, mut host, mut source) = setup(&ordinary_files(), &[protocol, RERELEASE]);
            let profiles: Vec<_> = host
                .runtime
                .model_assets
                .as_ref()
                .unwrap()
                .catalog()
                .profiles()
                .collect();
            let profile = *profiles
                .iter()
                .find(|profile| profile.source_rules == protocol.rule_set())
                .unwrap();
            let healthy_profile = *profiles
                .iter()
                .find(|profile| profile.source_rules == RERELEASE.rule_set())
                .unwrap();
            send(
                &mut host,
                &mut source,
                1,
                &serverdata(RERELEASE, 91, 31, false, 0, 40),
            );
            let stream = rerelease_stream(
                layout(&host, 1),
                false,
                91,
                (b"maps/ready.bsp", &q2_files::world(b"wall")),
            );
            send(&mut host, &mut source, 1, &stream);
            settle(&mut host, &mut source);
            assert_admitted(
                &host,
                &source,
                1,
                true,
                (b"maps/ready.bsp", &q2_files::world(b"wall")),
            );
            send(&mut host, &mut source, 1, &snapshot(RERELEASE, 1));
            settle(&mut host, &mut source);
            assert!(history(&host, 1) > 0);
            let healthy_lifetime = host.runtime.server.clients[1].entity;
            let healthy_history = history(&host, 1);
            let healthy_applied = host.runtime.snapshots[1].as_ref().unwrap().applied;

            let mut accepted = serverdata(protocol, 91, 31, false, 0, 40);
            accepted.extend(config(protocol, 0, b"accepted title"));
            send(&mut host, &mut source, 0, &accepted);
            assert_eq!(
                host.runtime.client_resources[0]
                    .as_ref()
                    .unwrap()
                    .received_q2_server_data()
                    .unwrap()
                    .server_count,
                91
            );
            let old_lifetime = host.runtime.server.clients[0].entity;
            let errors = host.runtime.network.command_errors;
            let assets = host
                .runtime
                .model_assets
                .as_ref()
                .unwrap()
                .assets()
                .counts();
            let mut malformed = serverdata(
                protocol,
                92,
                if truncate { 2 } else { -2 },
                true,
                0x8000,
                60,
            );
            if truncate {
                malformed.pop();
            }
            send(&mut host, &mut source, 0, &malformed);
            settle(&mut host, &mut source);
            assert_eq!(host.runtime.network.command_errors, errors + 1);
            assert!(
                host.runtime
                    .network
                    .get(ClientId(0), Endpoint::Client)
                    .is_none()
            );
            assert!(host.runtime.client_resources[0].is_none());
            assert!(host.runtime.snapshots[0].is_none());
            assert_eq!(host.local_clients[0], None);
            assert!(host.local_worlds[0].is_none());
            assert!(source.servers[0].prefixed(b"begin ").is_empty());
            assert_eq!(
                host.runtime
                    .model_assets
                    .as_ref()
                    .unwrap()
                    .assets()
                    .counts(),
                assets
            );

            send(&mut host, &mut source, 1, &snapshot(RERELEASE, 2));
            settle(&mut host, &mut source);
            assert_eq!(host.local_clients[1], Some(ClientId(1)));
            assert_eq!(host.runtime.server.clients[1].entity, healthy_lifetime);
            assert_eq!(
                host.runtime.snapshots[1].as_ref().unwrap().applied,
                healthy_applied + 1
            );
            assert!(history(&host, 1) > healthy_history);
            assert_eq!(source.servers[1].count(b"begin 91"), 1);
            assert_admitted(
                &host,
                &source,
                1,
                true,
                (b"maps/ready.bsp", &q2_files::world(b"wall")),
            );

            source.servers[0] = Server::new(ClientId(0), protocol);
            let replacement = source.servers[0].connect(&mut host.runtime);
            host.local_clients[0] = Some(replacement);
            assert_ne!(host.runtime.server.clients[0].entity, old_lifetime);
            assert_eq!(history(&host, 0), 0);
            host.bind_client_model_assets([
                (replacement, profile, None),
                (ClientId(1), healthy_profile, host.local_worlds[1]),
            ]);
            let mut header = serverdata(protocol, 93, 2, false, 0, 40);
            if protocol == CLASSIC {
                header.extend(stuff(protocol, b"cmd configstrings 93 0\n"));
            }
            send(&mut host, &mut source, 0, &header);
            settle(&mut host, &mut source);
            if protocol == CLASSIC {
                assert_eq!(source.servers[0].count(b"configstrings 93 0"), 1);
                let mut rows = world_rows(protocol, layout(&host, 0), true);
                rows.extend(stuff(protocol, b"cmd baselines 93 0\n"));
                send(&mut host, &mut source, 0, &rows);
                settle(&mut host, &mut source);
                assert_eq!(source.servers[0].count(b"baselines 93 0"), 1);
                let mut rows = baseline(protocol, false, 7);
                rows.extend(stuff(protocol, b"precache 93\n"));
                send(&mut host, &mut source, 0, &rows);
            } else {
                let stream = rerelease_stream(
                    layout(&host, 0),
                    false,
                    93,
                    (b"maps/ready.bsp", &q2_files::world(b"wall")),
                );
                send(&mut host, &mut source, 0, &stream);
            }
            settle(&mut host, &mut source);
            assert_admitted(
                &host,
                &source,
                0,
                true,
                (b"maps/ready.bsp", &q2_files::world(b"wall")),
            );
            assert_eq!(source.servers[0].count(b"begin 93"), 1);
            assert_eq!(history(&host, 0), 0);
            send(&mut host, &mut source, 0, &snapshot(protocol, 1));
            settle(&mut host, &mut source);
            assert!(history(&host, 0) > 0);
            assert_eq!(host.local_clients[1], Some(ClientId(1)));
            assert_eq!(host.runtime.server.clients[1].entity, healthy_lifetime);
            assert!(!host.runtime.quit);
        }
    }
}

#[test]
fn accepted_headers_and_cinematic_minus_one_preserve_native_metadata() {
    let (_fixture, mut host, mut source) = setup(&ordinary_files(), &[CLASSIC, RERELEASE]);
    for (client, protocol) in [CLASSIC, RERELEASE].into_iter().enumerate() {
        let mut bytes = serverdata(protocol, 91, 31, false, 0, 40);
        bytes.extend(config(protocol, 0, b"accepted title"));
        send(&mut host, &mut source, client, &bytes);
        let resources = host.runtime.client_resources[client].as_ref().unwrap();
        let data = resources.received_q2_server_data().unwrap();
        assert_eq!(
            (data.server_count, data.client_number, data.attract_loop),
            (91, 31, false)
        );
        assert_eq!(
            resources.state.configs.config(0).unwrap().text,
            b"accepted title"
        );
        assert_eq!(
            host.runtime.snapshots[client]
                .as_ref()
                .unwrap()
                .native_client(),
            Some(31)
        );
        assert!(
            !host.runtime.snapshots[client]
                .as_ref()
                .unwrap()
                .presentation_closed()
        );
    }
    send(
        &mut host,
        &mut source,
        0,
        &serverdata(CLASSIC, i32::MIN, -1, true, 0, 0),
    );
    settle(&mut host, &mut source);
    let data = host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .received_q2_server_data()
        .unwrap();
    assert_eq!(
        (data.server_count, data.client_number, data.attract_loop),
        (i32::MIN, -1, true)
    );
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        None
    );
    assert!(host.local_worlds[0].is_none());
    assert_eq!(history(&host, 0), 0);
    assert!(source.servers[0].prefixed(b"begin ").is_empty());
    let stream = rerelease_stream(
        layout(&host, 1),
        false,
        91,
        (b"maps/ready.bsp", &q2_files::world(b"wall")),
    );
    send(&mut host, &mut source, 1, &stream);
    settle(&mut host, &mut source);
    assert_admitted(
        &host,
        &source,
        1,
        true,
        (b"maps/ready.bsp", &q2_files::world(b"wall")),
    );
    assert_eq!(source.servers[1].count(b"begin 91"), 1);
    assert_eq!(history(&host, 0), 0);
    for state in [3, 5] {
        let before = history(&host, 1);
        let mut header = serverdata(RERELEASE, 100 + i32::from(state), 29, false, 0, 40);
        let at = header.len() - 4;
        header[at] = state;
        send(&mut host, &mut source, 1, &header);
        settle(&mut host, &mut source);
        let data = host.runtime.client_resources[1]
            .as_ref()
            .unwrap()
            .received_q2_server_data()
            .unwrap();
        assert_eq!(
            data.client_number, 29,
            "cinematic state does not change the retained signed header field"
        );
        assert_eq!(data.rerelease.unwrap().server_state, state);
        assert!(data.cinematic());
        assert_eq!(
            host.runtime.snapshots[1].as_ref().unwrap().native_client(),
            None
        );
        assert_eq!(history(&host, 1), before);
        assert_eq!(
            source.servers[1].count(format!("begin {}", 100 + i32::from(state)).as_bytes()),
            0
        );
    }
}

fn command(host: &mut FrameHost, source: &mut Source, client: usize, text: &str) {
    host.console
        .append(
            text,
            Context {
                source: source.servers[client].protocol.rule_set(),
                seat: SeatId::new(client as u8).unwrap(),
                ..Default::default()
            },
        )
        .unwrap();
    host.execute_commands_with_driver(&mut qa_app::modules::Driver::cache_only());
    step(host, source);
}
fn chunk(host: &mut FrameHost, source: &mut Source, client: usize, percent: u8, bytes: &[u8]) {
    let mut packet = [0; 1100];
    let length = write_chunk(
        source.servers[client].protocol,
        Chunk::Sequential {
            percent,
            bytes: Some(bytes),
            compressed: false,
        },
        &mut packet,
    )
    .unwrap();
    send(host, source, client, &packet[..length]);
}

#[test]
fn published_pcx_refresh_retries_backend_admission_without_downloading_again() {
    use qa_compat::services::ModelLookup;
    let (fixture, mut host, mut source) = setup(
        &[("models/ready.md2", support::md2_skin(b"models/later.pcx"))],
        &[CLASSIC],
    );
    let row = layout(&host, 0).resources[0].get(2).unwrap();
    send(
        &mut host,
        &mut source,
        0,
        &config(CLASSIC, row, b"models/ready.md2"),
    );
    settle(&mut host, &mut source);
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let profile = owner.catalog().profiles().next().unwrap();
    let handle = owner
        .catalog()
        .file_handle(b"models/ready.md2", None)
        .unwrap();
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let old = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap();
    let old_receipts = owner
        .catalog()
        .renderer_registrations(
            qa_compat::services::RendererRegistration::Model,
            handle,
            profile,
        )
        .unwrap()
        .to_vec();
    let prefix = owner.assets().counts();
    let revision = host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .configs
        .config(row)
        .unwrap()
        .revision;
    fixture.remove("models/ready.md2");
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    host.configure_client_downloads(ClientId(0), mount, mount, 8192, 64)
        .unwrap();
    for name in ["cl_allowDownload", "allow_download_models"] {
        let view = host
            .console
            .cvars
            .bind(
                name,
                Context {
                    source: RuleSetId::Quake2,
                    side: qa_console::catalog::Scope::Client,
                    ..Default::default()
                },
            )
            .unwrap();
        host.console.cvars.force_write(view, "1").unwrap();
    }
    command(&mut host, &mut source, 0, "download models/later.pcx\n");
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download models/later.pcx"), 1);
    source.reject_assets = true;
    let attempts = source.backend.attempts;
    chunk(&mut host, &mut source, 0, 100, &support::pcx(37));
    settle(&mut host, &mut source);
    assert!(source.backend.attempts > attempts);
    assert!(host.runtime.vfs.open(b"models/later.pcx").is_some());
    let owner = host.runtime.model_assets.as_ref().unwrap();
    assert_eq!(owner.assets().counts(), prefix);
    assert_eq!(source.backend.cpu.admitted_assets(), Some(prefix));
    assert_eq!(
        owner
            .catalog()
            .bindings(registration, 0, Default::default())
            .unwrap()
            .next(),
        Some(old)
    );
    assert_eq!(
        owner
            .catalog()
            .renderer_registrations(
                qa_compat::services::RendererRegistration::Model,
                handle,
                profile
            )
            .unwrap(),
        old_receipts
    );
    source.reject_assets = false;
    command(&mut host, &mut source, 0, "retrydownload\n");
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download models/later.pcx"), 1);
    assert_eq!(source.servers[0].count(b"nextdl"), 0);
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let current = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(current.model, old.model);
    assert_ne!(current.shader, old.shader);
    assert_eq!(owner.assets().counts().models, prefix.models);
    assert_eq!(
        source.backend.cpu.admitted_assets(),
        Some(owner.assets().counts())
    );
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(row)
            .unwrap()
            .revision,
        revision
    );
    let attempts = source.backend.attempts;
    settle(&mut host, &mut source);
    assert_eq!(source.backend.attempts, attempts);
}

#[test]
fn missing_wal_download_cancel_retry_holds_startup_while_the_other_client_begins() {
    let mut files = ordinary_files();
    files.retain(|(name, _)| *name != "textures/wall.wal");
    let healthy_world = q2_files::world(b"healthy");
    files.push(("maps/healthy.bsp", healthy_world.clone()));
    files.push(("textures/healthy.wal", q2_files::wal(33)));
    let (_fixture, mut host, mut source) = setup(&files, &[CLASSIC, RERELEASE]);
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    host.configure_client_downloads(ClientId(0), mount, mount, 8192, 64)
        .unwrap();
    for name in ["cl_allowDownload", "allow_download_maps"] {
        let view = host
            .console
            .cvars
            .bind(
                name,
                Context {
                    source: RuleSetId::Quake2,
                    side: qa_console::catalog::Scope::Client,
                    seat: SeatId::FIRST,
                    ..Default::default()
                },
            )
            .unwrap();
        host.console.cvars.force_write(view, "1").unwrap();
    }
    send(
        &mut host,
        &mut source,
        0,
        &serverdata(CLASSIC, 71, 0, false, 0, 0),
    );
    let mut rows = world_rows(CLASSIC, layout(&host, 0), false);
    rows.extend(stuff(CLASSIC, b"cmd baselines 71 0\n"));
    send(&mut host, &mut source, 0, &rows);
    settle(&mut host, &mut source);
    send(&mut host, &mut source, 0, &stuff(CLASSIC, b"precache 71\n"));
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download textures/wall.wal"), 1);
    assert!(source.servers[0].prefixed(b"begin ").is_empty());
    let wal = q2_files::wal(22);
    chunk(&mut host, &mut source, 0, 50, &wal[..100]);
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"nextdl"), 1);
    assert!(host.runtime.vfs.open(b"textures/wall.wal").is_none());
    command(&mut host, &mut source, 0, "stopdownload\n");
    chunk(&mut host, &mut source, 0, 100, &wal[100..]);
    settle(&mut host, &mut source);
    assert!(host.runtime.vfs.open(b"textures/wall.wal").is_none());
    assert!(source.servers[0].prefixed(b"begin ").is_empty());

    send(
        &mut host,
        &mut source,
        1,
        &serverdata(RERELEASE, 72, 1, false, 0, 40),
    );
    let stream = rerelease_stream(
        layout(&host, 1),
        false,
        72,
        (b"maps/healthy.bsp", &healthy_world),
    );
    send(&mut host, &mut source, 1, &stream);
    settle(&mut host, &mut source);
    assert_admitted(
        &host,
        &source,
        1,
        true,
        (b"maps/healthy.bsp", &healthy_world),
    );
    assert_eq!(source.servers[1].count(b"begin 72"), 1);
    assert!(source.servers[0].prefixed(b"begin ").is_empty());

    command(&mut host, &mut source, 0, "retrydownload\n");
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download textures/wall.wal"), 2);
    chunk(&mut host, &mut source, 0, 100, &wal);
    settle(&mut host, &mut source);
    let reference = host.runtime.vfs.open(b"textures/wall.wal").unwrap();
    let actual = host
        .runtime
        .vfs
        .read_to_vec(
            &reference,
            8192,
            &mut qa_formats::archive::ArchiveReader::default(),
        )
        .unwrap();
    assert_eq!(actual, wal);
    assert_admitted(
        &host,
        &source,
        0,
        false,
        (b"maps/ready.bsp", &q2_files::world(b"wall")),
    );
    assert_eq!(source.servers[0].count(b"begin 71"), 1);
    assert_eq!(source.servers[1].count(b"begin 72"), 1);
}

#[test]
fn q2_wire_checksum_selects_the_whole_file_for_q1_geometry_and_rejects_qw_lump_sum() {
    let world = support::world_map(16.0);
    let whole = qa_core::checksum::block_checksum(&world);
    let quakeworld = qa_formats::bsp::Bsp::parse(&world)
        .unwrap()
        .quakeworld_checksum2()
        .unwrap();
    assert_ne!(
        whole, quakeworld,
        "authored map must distinguish both native checksum contracts"
    );
    let files = [
        ("maps/foreign.bsp", world.clone()),
        ("progs/test.mdl", support::mdl()),
        ("sound/step.wav", sound_support::wav()),
    ];
    let (_fixture, mut host, mut source) = setup(&files, &[CLASSIC, CLASSIC]);
    source.backend.enabled = false;
    for (client, checksum) in [whole, quakeworld].into_iter().enumerate() {
        let before_history = history(&host, client);
        let generation = 81 + client as i32;
        send(
            &mut host,
            &mut source,
            client,
            &serverdata(CLASSIC, generation, client as i16, false, 0, 0),
        );
        let rows = layout(&host, client);
        let mut bytes = config(
            CLASSIC,
            rows.resources[0].get(1).unwrap(),
            b"maps/foreign.bsp",
        );
        bytes.extend(config(
            CLASSIC,
            rows.resources[0].get(2).unwrap(),
            b"progs/test.mdl",
        ));
        bytes.extend(config(
            CLASSIC,
            rows.resources[1].get(1).unwrap(),
            b"step.wav",
        ));
        bytes.extend(config(
            CLASSIC,
            rows.map_checksum().unwrap(),
            format!("{}", checksum as i32).as_bytes(),
        ));
        bytes.extend(stuff(
            CLASSIC,
            format!("cmd baselines {generation} 0\n").as_bytes(),
        ));
        send(&mut host, &mut source, client, &bytes);
        settle(&mut host, &mut source);
        assert_eq!(
            source.servers[client].count(format!("baselines {generation} 0").as_bytes()),
            1
        );
        send(
            &mut host,
            &mut source,
            client,
            &stuff(CLASSIC, format!("precache {generation}\n").as_bytes()),
        );
        settle(&mut host, &mut source);
        assert!(source.servers[client].prefixed(b"begin ").is_empty());
        assert_eq!(history(&host, client), before_history);
    }
    source.backend.enabled = true;
    settle(&mut host, &mut source);
    assert_admitted(&host, &source, 0, true, (b"maps/foreign.bsp", &world));
    let checksums = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .catalog()
        .world_native_checksums(b"maps/foreign.bsp")
        .unwrap();
    assert_eq!(
        (checksums.whole_file, checksums.quakeworld_lumps),
        (whole, Some(quakeworld))
    );
    assert_eq!(source.servers[0].count(b"begin 81"), 1);
    assert!(
        !host.runtime.snapshots[0]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert!(source.servers[1].prefixed(b"begin ").is_empty());
    assert!(
        host.runtime.snapshots[1]
            .as_ref()
            .is_none_or(|binding| binding.presentation_closed())
    );
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"begin 81"), 1);
}

fn compressed_download(percent: u8, deflate: &[u8]) -> Vec<u8> {
    message(|writer| {
        writer.write_bits(35, 8).unwrap();
        writer
            .write_bits(u32::from(u16::try_from(deflate.len()).unwrap()), 16)
            .unwrap();
        writer.write_bits(u32::from(percent), 8).unwrap();
        for &byte in deflate {
            writer.write_bits(u32::from(byte), 8).unwrap();
        }
    })
}

#[test]
fn rerelease_large_compressed_services_and_zero_output_download_retry_admit_the_world() {
    let mut files = ordinary_files();
    files.retain(|(name, _)| *name != "textures/wall.wal");
    let (_fixture, mut host, mut source) = setup(&files, &[RERELEASE]);
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    host.configure_client_downloads(ClientId(0), mount, mount, 8192, 64)
        .unwrap();
    for name in ["cl_allowDownload", "allow_download_maps"] {
        let view = host
            .console
            .cvars
            .bind(
                name,
                Context {
                    source: RuleSetId::Quake2Rerelease,
                    side: qa_console::catalog::Scope::Client,
                    seat: SeatId::FIRST,
                    ..Default::default()
                },
            )
            .unwrap();
        host.console.cvars.force_write(view, "1").unwrap();
    }
    send(
        &mut host,
        &mut source,
        0,
        &serverdata(RERELEASE, 401, 0, false, 0, 40),
    );
    let errors = host.runtime.network.command_errors;
    let expected = [b'r'; 95];
    let expanded_bytes = (12000..12100)
        .map(|row| config(RERELEASE, row, &expected).len())
        .sum::<usize>();
    assert!(expanded_bytes > 8192);
    // Python zlib level9/wbits=-15 golden for the 100 authored svc13 rows.
    // Decoding belongs solely to the ordinary native Commands path.
    let deflate = include_bytes!("fixtures/q2_services.deflate");
    let mut packet = message(|writer| {
        writer.write_bits(34, 8).unwrap();
        writer.write_bits(deflate.len() as u32, 16).unwrap();
        writer.write_bits(expanded_bytes as u32, 16).unwrap();
        for &byte in deflate {
            writer.write_bits(u32::from(byte), 8).unwrap();
        }
    });
    packet.extend(config(RERELEASE, 0, b"after compressed services"));
    send(&mut host, &mut source, 0, &packet);
    let resources = &host.runtime.client_resources[0].as_ref().unwrap().state;
    for row in 12000..12100 {
        assert_eq!(
            resources.configs.config(row).unwrap().text,
            expected,
            "expanded RR row {row}"
        );
    }
    assert_eq!(
        resources.configs.config(0).unwrap().text,
        b"after compressed services"
    );
    assert_eq!(host.runtime.network.command_errors, errors);
    assert!(source.servers[0].prefixed(b"begin ").is_empty());
    let mut rows = world_rows(RERELEASE, layout(&host, 0), false);
    rows.extend(stuff(RERELEASE, b"precache 401\n"));
    send(&mut host, &mut source, 0, &rows);
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download textures/wall.wal"), 1);

    // RFC1951: a nonfinal empty stored block consumes input but emits no file
    // bytes; the final stored WAL block continues that same inflate stream.
    let empty = compressed_download(10, &[0, 0, 0, 255, 255]);
    let wal = q2_files::wal(22);
    let length = u16::try_from(wal.len()).unwrap();
    let mut final_block = vec![1];
    final_block.extend(length.to_le_bytes());
    final_block.extend((!length).to_le_bytes());
    final_block.extend(&wal);
    let final_packet = compressed_download(100, &final_block);
    send(&mut host, &mut source, 0, &empty);
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"nextdl"), 1);
    assert!(host.runtime.vfs.open(b"textures/wall.wal").is_none());
    assert!(host.local_worlds[0].is_none());
    assert!(source.servers[0].prefixed(b"begin ").is_empty());
    assert!(
        !host.runtime.snapshots[0]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );

    command(&mut host, &mut source, 0, "stopdownload\n");
    send(&mut host, &mut source, 0, &final_packet);
    settle(&mut host, &mut source);
    assert!(host.runtime.vfs.open(b"textures/wall.wal").is_none());
    assert!(source.servers[0].prefixed(b"begin ").is_empty());
    command(&mut host, &mut source, 0, "retrydownload\n");
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download textures/wall.wal"), 2);
    send(&mut host, &mut source, 0, &empty);
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"nextdl"), 2);
    assert!(host.runtime.vfs.open(b"textures/wall.wal").is_none());
    let mut final_packet = final_packet;
    final_packet.extend(config(RERELEASE, 0, b"after compressed download"));
    send(&mut host, &mut source, 0, &final_packet);
    settle(&mut host, &mut source);
    let reference = host.runtime.vfs.open(b"textures/wall.wal").unwrap();
    let actual = host
        .runtime
        .vfs
        .read_to_vec(
            &reference,
            8192,
            &mut qa_formats::archive::ArchiveReader::default(),
        )
        .unwrap();
    assert_eq!(actual, wal);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(0)
            .unwrap()
            .text,
        b"after compressed download"
    );
    assert_eq!(host.runtime.network.command_errors, errors);
    assert_admitted(
        &host,
        &source,
        0,
        false,
        (b"maps/ready.bsp", &q2_files::world(b"wall")),
    );
    assert_eq!(source.servers[0].count(b"begin 401"), 1);
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"begin 401"), 1);
}
