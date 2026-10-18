#[path = "support/client_asset_frames.rs"]
mod asset_frames;
#[path = "support/frame_source.rs"]
mod frame_source;
#[path = "model_assets/support.rs"]
mod support;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    client_scene::ClientSceneInput,
    host::{FrameHost, FrameResult, FrameSource},
    model_assets::{ModelAssets, ModelRequest},
    snapshots::NativePlayerBinding,
};
use qa_compat::services::ModelLoader;
use qa_console::{commands::Console, views::Context};
use qa_core::{
    configstrings::ConfigLayout,
    loopback::Endpoint,
    presentation::{DisplayConfiguration, DisplayMode, RendererConfiguration, WindowRequest},
    primitives::*,
    sys_events::*,
};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection, ConnectionPhase, Route},
    outputs::{self, PeerUpdate},
    projection::{EntityProjection, PlayerProjection},
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use qa_render::{
    Assets, FrontEnd, Limits, PaletteId, Refdef,
    assets::{ImageId, StageTexture},
    scene::{Command, DrawKind, Viewport},
    shader::reference::ShaderView,
};
use qa_session::timing::TickRate;
use std::{ops::ControlFlow, sync::Arc, time::Duration};

const CLIENT: ClientId = ClientId(0);
const RULES: RuleSetId = RuleSetId::QuakeWorld;
const SOCKET: u16 = 19;
const SERVER: std::net::SocketAddr = std::net::SocketAddr::V4(std::net::SocketAddrV4::new(
    std::net::Ipv4Addr::LOCALHOST,
    26000,
));

struct NativeServer {
    client: ClientId,
    peer: std::net::SocketAddr,
    channel: Channel,
    commands: Commands,
    time: EventTime,
    received_commands: Vec<Vec<u8>>,
}
impl NativeServer {
    fn load(protocol: Protocol, client: ClientId) -> Self {
        let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
        channel.configure_snapshots(protocol).unwrap();
        channel.set_qport(27000 + client.0 as u16);
        let mut peer = SERVER;
        peer.set_port(peer.port() + client.0 as u16);
        Self {
            client,
            peer,
            channel,
            commands: Commands::load(protocol, 8192),
            time: EventTime::default(),
            received_commands: Vec::new(),
        }
    }

    fn receive(&mut self, bytes: &[u8]) {
        let received = self.channel.receive(bytes, self.time).unwrap();
        if let Delivery::Payload(payload) = received.delivery
            && (!payload.is_empty() || self.commands.protocol == Protocol::QuakeWorld28)
        {
            let sequence = received.header.sequence;
            let length = self.commands.stage(payload).unwrap();
            let received_commands = &mut self.received_commands;
            self.commands
                .decode(
                    length,
                    sequence,
                    self.time.milliseconds() as i32,
                    &mut self.channel,
                    None,
                    |incoming| {
                        if let qa_network::ingress::Incoming::CommandString { text, .. } = incoming
                        {
                            received_commands.push(text.to_vec());
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
                                from: Peer::Socket(self.peer),
                                bytes,
                            },
                        })
                        .is_ok())
                    .unwrap()
                    .is_some()
            );
            if self.commands.protocol != Protocol::QuakeWorld28 && !self.channel.pending_fragments()
            {
                break;
            }
        }
    }
}

struct Source {
    assets: asset_frames::Source,
    configuration: Arc<RendererConfiguration>,
    window: WindowRequest,
    pending: Option<WindowRequest>,
    time: u64,
    reject_publication: bool,
    publication_attempts: usize,
    source_palette: PaletteId,
    servers: Vec<NativeServer>,
    reject_assets: bool,
    asset_attempts: usize,
}
impl qa_app::renderer::AssetPreparer for Source {
    fn prepare_assets(&mut self, assets: &Assets, limits: Option<Limits>) -> Result<(), String> {
        self.asset_attempts += 1;
        if self.reject_assets {
            return Err("reject player-skin preparation".to_owned());
        }
        qa_app::renderer::AssetPreparer::prepare_assets(&mut self.assets, assets, limits)
    }
}
impl FrameSource for Source {
    fn video_kind(&self) -> Option<qa_app::renderer::Kind> {
        Some(qa_app::renderer::Kind::Cpu)
    }
    fn video_window_request(&self) -> Option<WindowRequest> {
        Some(self.window)
    }
    fn prepare_video(
        &mut self,
        policy: qa_app::renderer::VideoPolicy,
        assets: &Assets,
        limits: Limits,
    ) -> Result<qa_app::renderer::PreparedVideo, String> {
        let mut configuration = self.configuration.as_ref().clone();
        configuration.display.width = policy.window.width;
        configuration.display.height = policy.window.height;
        configuration.display.window_aspect = policy.window.window_aspect;
        let candidate = qa_app::renderer::PreparedVideo::cpu(
            Arc::new(configuration),
            assets,
            limits,
            policy.cpu_bands,
        )?;
        self.pending = Some(policy.window);
        Ok(candidate)
    }
    fn publish_video(
        &mut self,
        candidate: qa_app::renderer::PreparedVideo,
    ) -> Result<Arc<RendererConfiguration>, String> {
        let window = self.pending.take().ok_or("missing candidate window")?;
        self.publication_attempts += 1;
        if self.reject_publication {
            return Err("reject translated-skin publication".to_owned());
        }
        let (backend, configuration) = candidate.into_cpu()?;
        self.assets.cpu = backend;
        self.window = window;
        self.configuration = Arc::clone(&configuration);
        Ok(configuration)
    }
    fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
        Some(self)
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
    fn send_packet(&mut self, socket: u16, to: std::net::SocketAddr, bytes: &[u8]) -> bool {
        assert_eq!(socket, SOCKET);
        self.servers
            .iter_mut()
            .find(|server| server.peer == to)
            .expect("owned fixture server")
            .receive(bytes);
        true
    }
    fn render(
        &mut self,
        scene: Option<&mut qa_app::renderer::SceneComposer>,
        resources: Option<qa_app::model_assets::ModelView<'_>>,
        native: &dyn qa_render::shader::reference::NativeShaders,
        _: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
        _: &[Option<ClientSceneInput<'_>>; SeatId::COUNT],
        _: &[Option<qa_app::renderer::hud::HudView<'_>>; SeatId::COUNT],
        _: qa_console::presentation::ConsoleView<'_>,
    ) {
        let (Some(scene), Some(resources)) = (scene, resources) else {
            return;
        };
        if let Some(mut packet) = scene.finish_frame() {
            packet.resolve_shaders(resources.assets, native);
            self.assets.cpu.render(&packet, resources.assets);
            assert!(scene.recycle(packet).is_ok());
        }
    }
}

fn setting(host: &mut FrameHost, name: &str, value: &str) {
    let view = host
        .console
        .cvars
        .bind(
            name,
            Context {
                source: protocol(host).rule_set(),
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(view, value).unwrap();
}

fn loading() -> (support::Fixture, FrameHost, Source) {
    loading_model(RULES, support::mdl(), &[], false)
}

fn loading_model(
    rules: RuleSetId,
    player: Vec<u8>,
    extra: &[(&str, Vec<u8>)],
    foreign_palette: bool,
) -> (support::Fixture, FrameHost, Source) {
    let mut files = vec![
        ("maps/player.bsp", support::world_map(8.0)),
        ("progs/player.mdl", player.clone()),
        ("progs/ordinary.mdl", player),
        ("skins/local.pcx", support::pcx_dimensions(296, 194, 16)),
        ("skins/base.pcx", support::pcx_dimensions(296, 194, 96)),
        ("skins/broken.pcx", b"not a PCX".to_vec()),
    ];
    files.extend(extra.iter().cloned());
    let mut fixture = support::Fixture::new(&files);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let target = if foreign_palette {
        support::shifted_palette(&mut assets, 17)
    } else {
        palette
    };
    let profile = qa_app::model_assets::ModelProfile {
        palette: Some(target),
        ..ModelRequest::native("", rules, Some(palette)).profile()
    };
    let protocol = if rules == RuleSetId::Quake {
        Protocol::NetQuake15
    } else {
        Protocol::QuakeWorld28
    };
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let loaded = qa_app::map::read(runtime.vfs.read_view(None), "maps/player.bsp")
        .unwrap()
        .load(
            &runtime.vfs,
            &mut assets,
            &mut runtime.geometry,
            Some(palette),
            &[
                qa_render::material::world_load::WorldPresentationProfile::native(
                    rules,
                    Some(target),
                )
                .into(),
            ],
            Default::default(),
            false,
        )
        .unwrap();
    let world = loaded.render.world;
    let presentation = loaded.render.presentations[0];
    let mut owner =
        ModelAssets::new(&runtime.vfs, assets, [profile], [world], Default::default()).unwrap();
    owner
        .bind_loaded_world(
            &runtime.vfs,
            "maps/player.bsp",
            world,
            Some(loaded.collision),
            loaded.collision_bounds,
            None,
            None,
        )
        .unwrap();
    for path in [b"progs/player.mdl".as_slice(), b"progs/ordinary.mdl"] {
        owner.register_model(&runtime.vfs, path, rules).unwrap();
    }
    let (scene, _) = qa_app::renderer::SceneComposer::load(
        Limits::default(),
        owner.assets_mut(),
        vec![loaded.render],
    )
    .unwrap();
    runtime.scene = Some(scene);
    runtime.model_assets = Some(owner);
    let mut server = NativeServer::load(protocol, CLIENT);
    let mut channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
    channel.set_qport(27000);
    let client = runtime
        .connect_client(
            SeatId::FIRST,
            ClientPolicy {
                input: rules,
                client: rules,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake2,
            },
            NativePlayerBinding::Supplied(0),
            Connection {
                phase: ConnectionPhase::Connected,
                route: Route {
                    socket: SOCKET,
                    peer: Peer::Socket(SERVER),
                },
                channel,
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
            None,
        )
        .unwrap();
    assert_eq!(client, CLIENT);
    assert!(!runtime.server.clients[0].server_authoritative());
    assert!(runtime.network.get(CLIENT, Endpoint::Server).is_none());
    let resources = runtime.client_resources[0].as_mut().unwrap();
    let layout = ConfigLayout::for_rules(rules);
    resources
        .state
        .configs
        .set(layout.resources[0].get(1).unwrap(), b"maps/player.bsp")
        .unwrap();
    for (ordinal, path) in [
        (2, b"progs/player.mdl".as_slice()),
        (3, b"progs/ordinary.mdl"),
    ] {
        assert!(resources.apply(layout.resources[0].get(ordinal).unwrap(), path, None, None));
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(CLIENT);
    host.local_worlds[0] = Some((world, presentation.id));
    host.local_presentations[0] = presentation.defaults;
    host.local_presentation_rules[0] = Some(rules);
    if rules == RuleSetId::Quake {
        let mut header = vec![11];
        header.extend(15u32.to_le_bytes());
        header.extend([2, 0]);
        header
            .extend(b"colored peers\0maps/player.bsp\0progs/player.mdl\0progs/ordinary.mdl\0\0\0");
        send(&mut host, &mut server, &header);
    }
    setting(&mut host, "cl_noskins", "0");
    setting(&mut host, "baseskin", "base");
    setting(&mut host, "gl_nocolors", "0");
    setting(&mut host, "gl_playermip", "0");
    host.register_client_model_assets(None);
    let backend = asset_frames::Source::load(host.runtime.model_assets.as_ref().unwrap().assets());
    host.publish_model_admission(backend.cpu.admitted_assets().unwrap())
        .unwrap();
    host.runtime
        .model_assets
        .as_mut()
        .unwrap()
        .end_initial_registration();
    host.bind_client_model_assets([(CLIENT, profile, Some((world, presentation.id)))]);
    if rules == RuleSetId::Quake {
        assert_bound_models(&host, "initial NQ bind");
    }
    let configuration = Arc::new(RendererConfiguration {
        display: DisplayConfiguration {
            width: 64,
            height: 48,
            ..Default::default()
        },
        ..Default::default()
    });
    host.runtime.renderer_configuration = Some(Arc::clone(&configuration));
    let display = DisplayMode {
        width: 64,
        height: 48,
        frequency: 60,
        color_bits: 32,
    };
    host.console
        .cvars
        .set_video_policy(
            Arc::new(qa_core::presentation::VideoCapabilities {
                desktop: display,
                fullscreen: Box::default(),
            }),
            display,
        )
        .unwrap();
    host.set_cpu_band_override(Some(qa_render::cpu::RasterBands::One));
    for (name, value) in [
        ("r_cpuBands", "1"),
        ("r_mode", "-1"),
        ("r_customwidth", "64"),
        ("r_customheight", "48"),
        ("r_fullscreen", "0"),
    ] {
        let handle = host.console.cvars.find(name).unwrap();
        host.console.cvars.set_text(handle, value).unwrap();
    }
    let source = Source {
        assets: backend,
        configuration,
        window: WindowRequest::windowed(64, 48),
        pending: None,
        time: 100,
        reject_publication: false,
        publication_attempts: 0,
        source_palette: palette,
        servers: vec![server],
        reject_assets: false,
        asset_attempts: 0,
    };
    (fixture, host, source)
}

fn drain(host: &mut FrameHost, time: EventTime) {
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut FrameResult::default());
    host.client_modules();
}

fn peers(host: &mut FrameHost, source: &mut Source, updates: &[PeerUpdate<'_>]) {
    let protocol = protocol(host);
    let mut bytes = [0; 1400];
    let mut length = 0;
    for &update in updates {
        length += outputs::peer_update(protocol, update, &mut bytes[length..]).unwrap();
    }
    send(host, &mut source.servers[0], &bytes[..length]);
}

fn protocol(host: &FrameHost) -> Protocol {
    host.runtime.snapshots[0].as_ref().unwrap().protocol
}

fn send(host: &mut FrameHost, server: &mut NativeServer, bytes: &[u8]) {
    let time = EventTime(host.time.0 + 1_000_000);
    server.time = time;
    if server.commands.protocol == Protocol::QuakeWorld28 {
        let mut outgoing = frame_source::Sink::default();
        assert!(host.runtime.send_client_command(
            server.client,
            &UserCmd::default(),
            time,
            &mut outgoing
        ));
        for packet in outgoing.packets {
            assert!(packet.admitted);
            server.receive(&packet.bytes);
        }
    }
    server.channel.queue_reliable(bytes).unwrap();
    server.flush(&mut host.queue);
    drain(host, time);
    let mut outgoing = frame_source::Sink::default();
    host.runtime
        .send_client_command(server.client, &UserCmd::default(), time, &mut outgoing);
    for packet in outgoing.packets {
        assert!(packet.admitted);
        server.receive(&packet.bytes);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CapturedImage {
    id: ImageId,
    first: u8,
    second: u8,
    size: [u32; 2],
    skeletal: bool,
    rgba: [u8; 4],
    rgba_second: [u8; 4],
    prepared: [u32; 2],
}

fn assert_same_image_contents(actual: [CapturedImage; 3], expected: [CapturedImage; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert_eq!(
            actual,
            CapturedImage {
                id: actual.id,
                ..expected
            }
        );
    }
}

fn draw(host: &FrameHost, software: bool) -> [CapturedImage; 3] {
    draw_at(host, software, 1.0)
}

fn draw_at(host: &FrameHost, software: bool, time: f64) -> [CapturedImage; 3] {
    draw_colormaps(host, software, time, [1, 2, 1])
}

fn draw_colormaps(
    host: &FrameHost,
    software: bool,
    time: f64,
    colormaps: [u32; 3],
) -> [CapturedImage; 3] {
    draw_client_colormaps(host, CLIENT, software, time, colormaps)
}

fn draw_client_colormaps(
    host: &FrameHost,
    client: ClientId,
    software: bool,
    time: f64,
    colormaps: [u32; 3],
) -> [CapturedImage; 3] {
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let assets = owner.assets();
    let protocol = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .protocol;
    let rules = protocol.rule_set();
    let projection = EntityProjection::load(protocol);
    let player_projection = PlayerProjection::load(protocol, &[]);
    let records =
        [(2, colormaps[0]), (2, colormaps[1]), (3, colormaps[2])].map(|(ordinal, colormap)| {
            EntityRecord {
                position: Vec3([32., 0., 0.]),
                appearance: EntityAppearance {
                    source: rules,
                    models: [
                        ModelRef::native(ResourceNamespace::Client(client), ordinal),
                        None,
                        None,
                        None,
                    ],
                    native: NativeAppearanceTail {
                        colormap,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            }
        });
    let nq = entity_rows::<{ states::NQ_ENTITY_WORDS }>(&projection, &records);
    let qw = entity_rows::<{ states::QW_ENTITY_WORDS }>(&projection, &records);
    if protocol == Protocol::NetQuake15 {
        let resources = host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap();
        for (row, expected) in nq.iter().zip(&records) {
            let mut decoded = EntityRecord::default();
            assert!(projection.apply(&row.words, &mut decoded, ResourceNamespace::Client(client)));
            assert_eq!(
                decoded.appearance.models, expected.appearance.models,
                "NQ model roundtrip"
            );
            assert_eq!(
                decoded.appearance.native.colormap, expected.appearance.native.colormap,
                "NQ colormap roundtrip"
            );
            assert_eq!(
                decoded.appearance.frame, expected.appearance.frame,
                "NQ frame roundtrip"
            );
            assert_eq!(
                decoded.appearance.skin, expected.appearance.skin,
                "NQ skin roundtrip"
            );
            assert_eq!(
                decoded.appearance.alpha, expected.appearance.alpha,
                "NQ implicit alpha"
            );
            assert_eq!(
                decoded.appearance.scale, expected.appearance.scale,
                "NQ implicit scale"
            );
            let capabilities =
                qa_gameplay::rules::appearance::policy(rules).project(&decoded.appearance);
            let material = qa_app::model_assets::ModelMaterialPolicy {
                translucent: capabilities.alpha != 1.0,
                fullbright: capabilities.fullbright,
                custom_skin: 0,
            };
            assert!(
                resources
                    .state
                    .models
                    .resolve(
                        decoded.appearance.models[0].unwrap(),
                        decoded.appearance.skin,
                        owner.catalog(),
                        material,
                    )
                    .is_some(),
                "NQ decoded model policy {material:?}"
            );
        }
    }
    let nq_player = [0; states::NQ_PLAYER_WORDS];
    let current = if protocol == Protocol::NetQuake15 {
        ReceivedFrame::NetQuake(Frame {
            sequence: 1,
            time: ThinkTime::Seconds(time),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &nq_player,
            entities: &nq,
        })
    } else {
        ReceivedFrame::QuakeWorld(Frame {
            sequence: 1,
            time: ThinkTime::Seconds(time),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &[],
            entities: &qw,
        })
    };
    let player = PlayerState::default();
    let resources = host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap();
    let input = ClientSceneInput {
        synchronization: None,
        client,
        current: Some(current),
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(rules),
        player_projection: &player_projection,
        bindings: &resources.state.models,
        models: owner.catalog(),
        static_entities: &[],
        player: &player,
        weapon_view_angles: Vec3::default(),
        alias_roll: Default::default(),
        interpolation: Default::default(),
        gun_animation: None,
        gun_previous: None,
        view_entity: None,
    };
    let mut frontend = FrontEnd::load(Limits::default().for_assets(assets).unwrap()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    let refdef = Refdef {
        viewport: Viewport {
            x: 0,
            y: 0,
            width: 64,
            height: 48,
        },
        model_time: Some(ThinkTime::Seconds(time)),
        skin_timing: if software {
            qa_render::scene::SkinTiming::Cumulative
        } else {
            qa_render::scene::SkinTiming::LegacyFour
        },
        ..Default::default()
    };
    let counts = input.append(&mut frame, assets, &refdef, software, None);
    assert_eq!(
        (counts.models, counts.missing_models, counts.rejected),
        (3, 0, 0),
        "{protocol:?} colormaps={colormaps:?}",
    );
    assert!(frame.render_scene(refdef, &[], assets, ShaderView::direct()));
    let packet = frame.finish();
    let mut result = [None; 3];
    for command in packet.commands() {
        let Command::View(view) = command else {
            continue;
        };
        for item in packet.draws(view.scene.draws) {
            if item.kind != DrawKind::Entity {
                continue;
            }
            let shader = item.shader.unwrap().resolve(assets, &()).unwrap();
            let binding = shader.resources(assets).unwrap();
            let material = assets.material(binding.material).unwrap();
            let StageTexture::Image(id) = material.stages[0].texture else {
                panic!("indexed skin image")
            };
            let image = assets.image(id).unwrap();
            let prepared = &image.prepared.as_ref().unwrap().levels[0];
            result[item.index as usize] = Some(CapturedImage {
                id,
                first: image.indexed.as_ref().unwrap().mip(0).unwrap().indices()[0],
                second: image.indexed.as_ref().unwrap().mip(0).unwrap().indices()[1],
                size: image.logical_size(),
                skeletal: assets
                    .model(item.mesh.unwrap())
                    .unwrap()
                    .skeleton()
                    .is_some(),
                rgba: image.rgba[..4].try_into().unwrap(),
                rgba_second: image.rgba[4..8].try_into().unwrap(),
                prepared: [prepared.width, prepared.height],
            });
        }
    }
    result.map(Option::unwrap)
}

fn entity_rows<const N: usize>(
    projection: &EntityProjection,
    records: &[EntityRecord; 3],
) -> [Entity<N>; 3] {
    std::array::from_fn(|index| {
        let number = index as u32 + 41;
        let mut words = [0; N];
        if projection.words() == N {
            assert!(projection.reduce(
                &records[index],
                &Default::default(),
                &mut words,
                |model, _| Some(model.ordinal),
                |sound| Some(sound.ordinal),
                |reference| Some(reference.ordinal()),
            ));
            if projection.rule_set() == RuleSetId::Quake {
                let target: &[u32; states::NQ_ENTITY_WORDS] = words.as_slice().try_into().unwrap();
                let mut wire = [0; 128];
                let mut writer = qa_network::message::Writer::new(
                    &mut wire,
                    qa_network::message::Encoding::Bytes,
                );
                assert!(
                    states::write_nq_entity(
                        &mut writer,
                        number,
                        &[0; states::NQ_ENTITY_WORDS],
                        target,
                        false,
                    )
                    .unwrap()
                );
                let mut reader = qa_network::message::Reader::new(
                    writer.bytes(),
                    qa_network::message::Encoding::Bytes,
                );
                let accepted =
                    states::read_nq_entity(&mut reader, &[0; states::NQ_ENTITY_WORDS]).unwrap();
                assert_eq!(u32::from(accepted.number), number);
                words.copy_from_slice(&accepted.words.unwrap());
            }
        }
        Entity { number, words }
    })
}

fn prepare(host: &mut FrameHost, source: &mut Source) {
    host.register_client_model_assets(Some(source));
}

fn assert_bound_models(host: &FrameHost, phase: &str) {
    let resources = host.runtime.client_resources[0].as_ref().unwrap();
    let catalog = host.runtime.model_assets.as_ref().unwrap().catalog();
    let layout = ConfigLayout::for_rules(protocol(host).rule_set());
    for (ordinal, path) in [
        (2, b"progs/player.mdl".as_slice()),
        (3, b"progs/ordinary.mdl"),
    ] {
        assert_eq!(
            resources
                .state
                .configs
                .config(layout.resources[0].get(ordinal).unwrap())
                .unwrap()
                .text,
            path,
            "{phase}: accepted native precache {ordinal}",
        );
        assert!(
            resources
                .state
                .models
                .resolve(
                    ModelRef::native(ResourceNamespace::Client(CLIENT), ordinal as u32).unwrap(),
                    0,
                    catalog,
                    Default::default(),
                )
                .is_some(),
            "{phase}: native precache {ordinal} must have a published model binding"
        );
    }
}

#[test]
fn received_qw_peers_translate_only_the_registered_player_and_keep_local_skins() {
    let (_fixture, mut host, mut source) = loading();
    peers(
        &mut host,
        &mut source,
        &[
            PeerUpdate::UserInfo {
                slot: 0,
                user_id: 101,
                info: b"\\name\\One\\skin\\local\\topcolor\\2\\bottomcolor\\3",
            },
            PeerUpdate::UserInfo {
                slot: 1,
                user_id: 202,
                info: b"\\name\\Two\\skin\\local\\topcolor\\4\\bottomcolor\\5",
            },
        ],
    );
    prepare(&mut host, &mut source);
    let translated = draw(&host, true);
    assert_eq!((translated[0].first, translated[1].first), (32, 64));
    assert_ne!(translated[0].id, translated[1].id);
    assert_eq!(translated[0].size, [320, 200]);
    assert_eq!(translated[2].size, [2, 1]);
    assert_eq!(translated[2].first, 1);
    let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
    assert_eq!(state.peer(0).unwrap().metadata.user_id, Some(101));
    assert_eq!(state.peer(1).unwrap().metadata.user_id, Some(202));

    setting(&mut host, "cl_noskins", "1");
    prepare(&mut host, &mut source);
    let native = draw(&host, true);
    assert_eq!((native[0].size, native[1].size), ([2, 1], [2, 1]));
    assert_ne!(native[0].id, translated[0].id);
    assert_eq!(native[2], translated[2]);
    setting(&mut host, "cl_noskins", "1.5");
    prepare(&mut host, &mut source);
    assert_eq!(
        draw(&host, true),
        translated,
        "only exact1 disables local QW skins"
    );
    setting(&mut host, "cl_noskins", "2");
    prepare(&mut host, &mut source);
    assert_eq!(draw(&host, true), translated);

    setting(&mut host, "gl_nocolors", "0.5");
    prepare(&mut host, &mut source);
    assert_eq!(
        draw(&host, true),
        translated,
        "software ignores the GL color switch"
    );
    let gl = draw(&host, false);
    assert_eq!((gl[0].size, gl[1].size), ([2, 1], [2, 1]));

    host.console
        .append(
            "allskins base\n",
            Context {
                source: RULES,
                ..Default::default()
            },
        )
        .unwrap();
    host.execute_commands_with_driver(&mut qa_app::modules::Driver::cache_only());
    prepare(&mut host, &mut source);
    let forced = draw(&host, true);
    assert_eq!((forced[0].first, forced[1].first), (48, 80));
    assert_eq!(forced[2], translated[2]);
    host.console
        .append(
            "allskins\n",
            Context {
                source: RULES,
                ..Default::default()
            },
        )
        .unwrap();
    host.execute_commands_with_driver(&mut qa_app::modules::Driver::cache_only());
    prepare(&mut host, &mut source);
    let restored = draw(&host, true);
    assert_same_image_contents(restored, translated);
    assert_eq!(restored[2], translated[2]);
}

#[test]
fn missing_qw_skin_uses_base_but_present_malformed_skin_uses_the_model() {
    let (_fixture, mut host, mut source) = loading();
    peers(
        &mut host,
        &mut source,
        &[
            PeerUpdate::UserInfo {
                slot: 0,
                user_id: 101,
                info: b"\\name\\Missing\\skin\\absent\\topcolor\\2\\bottomcolor\\3",
            },
            PeerUpdate::UserInfo {
                slot: 1,
                user_id: 202,
                info: b"\\name\\Broken\\skin\\broken\\topcolor\\4\\bottomcolor\\5",
            },
        ],
    );
    prepare(&mut host, &mut source);
    let images = draw(&host, true);
    assert_eq!((images[0].size, images[0].first), ([320, 200], 48));
    assert_eq!((images[1].size, images[1].first), ([2, 1], 1));
    assert_eq!((images[2].size, images[2].first), ([2, 1], 1));
    assert!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .rejected
            > 0
    );
}

#[test]
fn video_reload_rebinds_received_qw_skin_selections_to_replayed_images() {
    let (_fixture, mut host, mut source) = loading();
    peers(
        &mut host,
        &mut source,
        &[
            PeerUpdate::UserInfo {
                slot: 0,
                user_id: 101,
                info: b"\\name\\One\\skin\\local\\topcolor\\2\\bottomcolor\\3",
            },
            PeerUpdate::UserInfo {
                slot: 1,
                user_id: 202,
                info: b"\\name\\Two\\skin\\local\\topcolor\\4\\bottomcolor\\5",
            },
        ],
    );
    let owner = (
        CLIENT,
        host.runtime.server.clients[CLIENT.0 as usize].entity,
    );
    let view = host
        .console
        .cvars
        .bind(
            "gl_playermip",
            Context {
                source: RULES,
                client_lifetime: Some(owner),
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(view, "1").unwrap();
    let global = host
        .console
        .cvars
        .bind(
            "gl_playermip",
            Context {
                source: RULES,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(host.console.cvars.integer_view(global).unwrap(), 0);
    prepare(&mut host, &mut source);
    let before = draw(&host, true);
    assert_eq!(before[0].prepared, [256, 128]);
    let maximum = host.console.cvars.find("gl_max_size").unwrap();
    host.console.cvars.set_text(maximum, "64").unwrap();
    let revision = host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .models
        .revision();
    host.console
        .append(
            "vid_restart cpu\n",
            Context {
                source: RULES,
                ..Default::default()
            },
        )
        .unwrap();
    host.execute_commands_with_driver(&mut qa_app::modules::Driver::cache_only());
    host.frame(&mut source, true);
    let after = draw(&host, true);
    assert_eq!((after[0].first, after[1].first), (32, 64));
    assert_ne!(
        after[0].id, before[0].id,
        "reload recompiles the retained skin request"
    );
    assert_ne!(after[1].id, before[1].id);
    assert_eq!(after[0].size, before[0].size);
    assert_eq!(after[1].size, before[1].size);
    assert_eq!(
        after[0].prepared,
        [32, 32],
        "new profile cap precedes player mip"
    );
    assert_eq!(after[1].prepared, [32, 32]);
    let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
    assert_eq!(state.models.revision(), revision);
    assert_eq!(state.peer(0).unwrap().metadata.user_id, Some(101));
    assert_eq!(state.peer(1).unwrap().metadata.user_id, Some(202));
}

fn colored_mdl(grouped: bool) -> Vec<u8> {
    let original = support::mdl();
    let mut bytes = original[..84].to_vec();
    bytes.extend(i32::from(grouped).to_le_bytes());
    if grouped {
        bytes.extend(2i32.to_le_bytes());
        bytes.extend(0.25f32.to_le_bytes());
        bytes.extend(1.0f32.to_le_bytes());
        bytes.extend([16, 96, 17, 97]);
    } else {
        bytes.extend([16, 96]);
    }
    bytes.extend(&original[90..]);
    bytes
}

fn colored_peers(host: &mut FrameHost, source: &mut Source) {
    if protocol(host) == Protocol::NetQuake15 {
        peers(
            host,
            source,
            &[
                PeerUpdate::Name {
                    slot: 0,
                    name: b"One",
                },
                PeerUpdate::Colors {
                    slot: 0,
                    colors: 0x23,
                },
                PeerUpdate::Name {
                    slot: 1,
                    name: b"Two",
                },
                PeerUpdate::Colors {
                    slot: 1,
                    colors: 0x45,
                },
            ],
        );
    } else {
        peers(
            host,
            source,
            &[
                PeerUpdate::UserInfo {
                    slot: 0,
                    user_id: 101,
                    info: b"\\name\\One\\skin\\local\\topcolor\\2\\bottomcolor\\3",
                },
                PeerUpdate::UserInfo {
                    slot: 1,
                    user_id: 202,
                    info: b"\\name\\Two\\skin\\local\\topcolor\\4\\bottomcolor\\5",
                },
            ],
        );
    }
}

fn restart(host: &mut FrameHost, source: &mut Source) {
    host.console
        .append(
            "vid_restart cpu\n",
            Context {
                source: protocol(host).rule_set(),
                ..Default::default()
            },
        )
        .unwrap();
    host.execute_commands_with_driver(&mut qa_app::modules::Driver::cache_only());
    host.frame(source, true);
}

#[test]
fn noskins_disables_external_qw_files_but_retains_embedded_player_colors() {
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(false), &[], false);
    colored_peers(&mut host, &mut source);
    prepare(&mut host, &mut source);
    let external = draw(&host, true);
    assert_eq!(external[0].size, [320, 200]);
    setting(&mut host, "cl_noskins", "1");
    prepare(&mut host, &mut source);
    let embedded = draw(&host, true);
    assert_eq!((embedded[0].first, embedded[0].second), (32, 48));
    assert_eq!((embedded[1].first, embedded[1].second), (64, 80));
    assert_eq!(embedded[0].size, [2, 1]);
    assert_ne!(embedded[0].id, external[0].id);
    assert!(!embedded[0].skeletal);

    peers(
        &mut host,
        &mut source,
        &[PeerUpdate::SetInfo {
            slot: 0,
            key: b"topcolor",
            value: b"6",
        }],
    );
    prepare(&mut host, &mut source);
    let changed = draw(&host, true);
    assert_eq!((changed[0].first, changed[0].second), (96, 48));
    assert_ne!(changed[0].id, embedded[0].id);
    assert_eq!(changed[1], embedded[1]);
    setting(&mut host, "gl_nocolors", "1");
    prepare(&mut host, &mut source);
    assert_eq!(draw(&host, true), changed);
    let plain = draw_colormaps(&host, false, 1.0, [0, 0, 0]);
    let gl = draw(&host, false);
    assert_eq!(gl[0], plain[0]);
    assert_eq!(gl[1], plain[1]);
    assert_eq!((gl[0].first, gl[0].second), (16, 96));
}

#[test]
fn translated_embedded_groups_keep_view_timing_and_survive_reload_rollback() {
    let (fixture, mut host, mut source) = loading_model(RULES, colored_mdl(true), &[], false);
    colored_peers(&mut host, &mut source);
    setting(&mut host, "cl_noskins", "1");
    prepare(&mut host, &mut source);
    let first = draw_at(&host, true, 0.125);
    let second = draw_at(&host, true, 0.25);
    assert_eq!((first[0].first, first[0].second), (32, 48));
    assert_eq!((second[0].first, second[0].second), (33, 49));
    assert_eq!((first[1].first, second[1].first), (64, 65));
    assert_ne!(first[0].id, second[0].id);
    let gl = draw_at(&host, false, 0.1);
    assert_eq!((gl[0].first, gl[0].second), (33, 49));
    assert_eq!(draw_at(&host, true, 0.1), first);
    let before = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    fixture.remove("progs/player.mdl");
    source.reject_publication = true;
    restart(&mut host, &mut source);
    assert_eq!(
        source.publication_attempts, 1,
        "rejection reaches the final publication boundary"
    );
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        before
    );
    assert_eq!(draw_at(&host, true, 0.125), first);
    assert_eq!(draw_at(&host, true, 0.25), second);
    source.reject_publication = false;
    restart(&mut host, &mut source);
    assert_eq!(source.publication_attempts, 2);
    let reloaded_first = draw_at(&host, true, 0.125);
    let reloaded_second = draw_at(&host, true, 0.25);
    assert_eq!(reloaded_first[0].id, first[0].id);
    assert_eq!(reloaded_second[0].id, second[0].id);
    assert_eq!(
        (reloaded_first[0].first, reloaded_first[0].second),
        (32, 48)
    );
    assert_eq!(
        (reloaded_second[0].first, reloaded_second[0].second),
        (33, 49)
    );
    assert_eq!(
        (reloaded_first[1].first, reloaded_second[1].first),
        (64, 65)
    );
}

#[test]
fn netquake_colormap_uses_received_colors_before_target_palette_conversion() {
    let (_fixture, mut host, mut source) =
        loading_model(RuleSetId::Quake, colored_mdl(false), &[], true);
    colored_peers(&mut host, &mut source);
    assert_bound_models(&host, "NQ peer receipt");
    let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
    for (slot, colors, top, bottom) in [(0, 0x23, 2, 3), (1, 0x45, 4, 5)] {
        let peer = state.peer(slot).unwrap().metadata;
        assert_eq!(peer.colors, Some(colors));
        assert_eq!((peer.top_color, peer.bottom_color), (top, bottom));
    }
    let no_skins = host
        .console
        .cvars
        .bind(
            "cl_noskins",
            Context {
                source: RuleSetId::Quake,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(host.console.cvars.numeric(no_skins).unwrap(), 0.0);
    let rejected = state.rejected;
    // Accepted gamestates retire the previous presentation through CLIENT output.
    host.dispatch_output(&mut source, &mut FrameResult::default());
    prepare(&mut host, &mut source);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .rejected,
        rejected,
        "NQ colors must prepare without a rejected source/backend transaction"
    );
    assert_bound_models(&host, "NQ cold preparation");
    let plain = draw_colormaps(&host, true, 1.0, [0, 0, 0]);
    let translated = draw(&host, true);
    // The target palette is the source palette rotated by17 entries.
    assert_eq!((translated[0].first, translated[0].second), (15, 31));
    assert_eq!((translated[1].first, translated[1].second), (47, 63));
    assert_eq!(translated[0].rgba, [32, 223, 16, 255]);
    assert_eq!(translated[1].rgba, [64, 191, 32, 255]);
    assert_eq!(
        (translated[2].first, translated[2].second),
        (15, 31),
        "ordinary NQ alias uses colormap1 too"
    );
    assert_eq!(translated[2].rgba, translated[0].rgba);
    assert_ne!(plain[0].id, translated[0].id);
    setting(&mut host, "gl_nocolors", "1");
    prepare(&mut host, &mut source);
    assert_eq!(draw(&host, true), translated);
    let gl = draw(&host, false);
    assert_eq!(gl[0], plain[0]);
    assert_eq!(gl[1], plain[1]);
    let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
    assert_eq!(state.peer(0).unwrap().metadata.colors, Some(0x23));
    assert_eq!(state.peer(1).unwrap().metadata.colors, Some(0x45));
    setting(&mut host, "gl_nocolors", "0");
    peers(
        &mut host,
        &mut source,
        &[PeerUpdate::Colors {
            slot: 0,
            colors: 0xef,
        }],
    );
    prepare(&mut host, &mut source);
    let high = draw(&host, true);
    let palette = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .palette(source.source_palette)
        .unwrap();
    assert_eq!(
        high[0].rgba,
        palette.color(239).to_le_bytes(),
        "NQ color14 reverses the top ramp"
    );
    assert_eq!(
        high[0].rgba_second,
        palette.color(255).to_le_bytes(),
        "NQ color15 reverses the bottom ramp"
    );
    assert_eq!(high[2].rgba, high[0].rgba);
    assert_eq!(high[2].rgba_second, high[0].rgba_second);
}

fn replacement_files() -> Vec<(&'static str, Vec<u8>)> {
    let lmp =
        |indices: [u8; 2]| [2u32.to_le_bytes().as_slice(), &1u32.to_le_bytes(), &indices].concat();
    vec![
        (
            "progs/player.md5mesh",
            br#"MD5Version 10
commandline "translated sidecar"
numJoints 1
numMeshes 1
joints { "origin" -1 ( 0 0 0 ) ( 0 0 0 ) }
mesh {
shader "shirt"
numverts 3
vert 0 ( 0 0 ) 0 1
vert 1 ( 1 0 ) 1 1
vert 2 ( 0 1 ) 2 1
numtris 1
tri 0 0 1 2
numweights 3
weight 0 0 1 ( 8 4 0 )
weight 1 0 1 ( 8 0 4 )
weight 2 0 1 ( 8 0 0 )
}
"#
            .to_vec(),
        ),
        (
            "progs/player.md5anim",
            br#"MD5Version 10
commandline "translated sidecar"
numFrames 2
numJoints 1
frameRate 24
numAnimatedComponents 1
hierarchy { "origin" -1 2 0 }
bounds { ( 0 0 0 ) ( 16 16 16 ) ( 0 0 0 ) ( 16 16 16 ) }
baseframe { ( 0 0 0 ) ( 0 0 0 ) }
frame 0 { 0 }
frame 1 { 2 }
"#
            .to_vec(),
        ),
        ("progs/shirt_00_00.lmp", lmp([18, 98])),
        ("progs/shirt_00_01.lmp", lmp([19, 99])),
    ]
}

#[test]
fn embedded_color_choices_keep_the_selected_md5_sidecar_and_native_group_timing() {
    let files = replacement_files();
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(true), &files, false);
    colored_peers(&mut host, &mut source);
    setting(&mut host, "cl_noskins", "1");
    prepare(&mut host, &mut source);
    let first = draw_at(&host, true, 0.125);
    let second = draw_at(&host, true, 0.25);
    assert!(first[0].skeletal && first[1].skeletal);
    assert!(second[0].skeletal && second[1].skeletal);
    assert!(!first[2].skeletal);
    assert_eq!((first[0].first, first[0].second), (34, 50));
    assert_eq!((second[0].first, second[0].second), (35, 51));
    assert_eq!((first[1].first, second[1].first), (66, 67));
    assert_ne!(first[0].id, second[0].id);
    setting(&mut host, "gl_nocolors", "1");
    prepare(&mut host, &mut source);
    assert_eq!(draw_at(&host, true, 0.125), first);
    let gl = draw_at(&host, false, 0.1);
    assert!(gl[0].skeletal && gl[1].skeletal);
    assert_eq!((gl[0].first, gl[0].second), (19, 99));
}

#[path = "client_qw_skins/refresh.rs"]
mod refresh;

#[path = "client_qw_skins/downloads.rs"]
mod downloads;
