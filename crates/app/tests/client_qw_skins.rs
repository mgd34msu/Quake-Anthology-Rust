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
    map::SpawnAnchor,
    model_assets::{ModelAssets, ModelRequest},
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
    commands::packet::Protocol,
    outputs::{self, PeerUpdate},
    projection::{EntityProjection, PlayerProjection},
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use qa_render::{
    Assets, FrontEnd, Limits, Refdef,
    assets::{ImageId, StageTexture},
    scene::{Command, DrawKind, Viewport},
    shader::reference::ShaderView,
};
use qa_session::timing::TickRate;
use std::{sync::Arc, time::Duration};

const CLIENT: ClientId = ClientId(0);
const RULES: RuleSetId = RuleSetId::QuakeWorld;

struct Source {
    assets: asset_frames::Source,
    configuration: Arc<RendererConfiguration>,
    window: WindowRequest,
    pending: Option<WindowRequest>,
    time: u64,
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
        let (backend, configuration) = candidate.into_cpu()?;
        self.assets.cpu = backend;
        self.window = window;
        self.configuration = Arc::clone(&configuration);
        Ok(configuration)
    }
    fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
        Some(&mut self.assets)
    }
    fn begin_frame(&mut self) -> Duration {
        self.time += 100;
        Duration::from_millis(self.time)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
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
                source: RULES,
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(view, value).unwrap();
}

fn loading() -> (support::Fixture, FrameHost, Source) {
    let mut fixture = support::Fixture::new(&[
        ("maps/player.bsp", support::world_map(8.0)),
        ("progs/player.mdl", support::mdl()),
        ("progs/ordinary.mdl", support::mdl()),
        ("skins/local.pcx", support::pcx_dimensions(296, 194, 16)),
        ("skins/base.pcx", support::pcx_dimensions(296, 194, 96)),
        ("skins/broken.pcx", b"not a PCX".to_vec()),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", RULES, Some(palette)).profile();
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let loaded = qa_app::map::read(&runtime.vfs, "maps/player.bsp")
        .unwrap()
        .load(
            &runtime.vfs,
            &mut assets,
            &mut runtime.geometry,
            Some(palette),
            &[
                qa_render::material::world_load::WorldPresentationProfile::native(
                    RULES,
                    Some(palette),
                )
                .into(),
            ],
            Default::default(),
            false,
        )
        .unwrap();
    let world = loaded.render.world;
    let presentation = loaded.render.presentations[0];
    let mut owner = ModelAssets::new(assets, [profile], [world], Default::default()).unwrap();
    owner
        .bind_loaded_world(
            "maps/player.bsp",
            world,
            Some(loaded.collision),
            loaded.collision_bounds,
        )
        .unwrap();
    for path in [b"progs/player.mdl".as_slice(), b"progs/ordinary.mdl"] {
        owner.register_model(&runtime.vfs, path, RULES).unwrap();
    }
    let (scene, _) = qa_app::renderer::SceneComposer::load(
        Limits::default(),
        owner.assets_mut(),
        vec![loaded.render],
    )
    .unwrap();
    runtime.scene = Some(scene);
    runtime.model_assets = Some(owner);
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RULES,
                client: RULES,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake2,
            },
            Protocol::QuakeWorld28,
            0,
        )
        .unwrap();
    assert_eq!(client, CLIENT);
    let resources = runtime.client_resources[0].as_mut().unwrap();
    let layout = ConfigLayout::for_rules(RULES);
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
    host.local_presentation_rules[0] = Some(RULES);
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

fn peers(host: &mut FrameHost, updates: &[PeerUpdate<'_>]) {
    let time = EventTime(host.time.0 + 1_000_000);
    assert!(host.runtime.send_client_command(
        CLIENT,
        &UserCmd::default(),
        time,
        &mut frame_source::Sink::default()
    ));
    drain(host, time);
    let mut bytes = [0; 1400];
    let mut length = 0;
    for &update in updates {
        length +=
            outputs::peer_update(Protocol::QuakeWorld28, update, &mut bytes[length..]).unwrap();
    }
    let runtime = &mut host.runtime;
    let channel = &mut runtime
        .network
        .get_mut(CLIENT, Endpoint::Server)
        .unwrap()
        .channel;
    assert!(
        channel
            .prepare(Some(&bytes[..length]), time)
            .unwrap()
            .is_some()
    );
    assert!(
        channel
            .submit_with(time, |packet| runtime
                .loopback
                .send(Endpoint::Server, CLIENT, packet)
                .is_ok())
            .unwrap()
            .is_some()
    );
    drain(host, time);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CapturedImage {
    id: ImageId,
    first: u8,
    size: [u32; 2],
}

fn draw(host: &FrameHost, software: bool) -> [CapturedImage; 3] {
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let assets = owner.assets();
    let projection = EntityProjection::load(Protocol::QuakeWorld28);
    let player_projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let rows = [(2, 1), (2, 2), (3, 1)].map(|(ordinal, colormap)| {
        let record = EntityRecord {
            position: Vec3([32., 0., 0.]),
            appearance: EntityAppearance {
                source: RULES,
                models: [
                    ModelRef::native(ResourceNamespace::Client(CLIENT), ordinal),
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
        };
        let mut words = [0; states::QW_ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &Default::default(),
            &mut words,
            |model, _| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal())
        ));
        Entity {
            number: 40 + colormap + if ordinal == 3 { 2 } else { 0 },
            words,
        }
    });
    let current = ReceivedFrame::QuakeWorld(Frame {
        sequence: 1,
        time: ThinkTime::Seconds(1.),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &[],
        entities: &rows,
    });
    let player = PlayerState::default();
    let resources = host.runtime.client_resources[0].as_ref().unwrap();
    let input = ClientSceneInput {
        synchronization: None,
        client: CLIENT,
        current: Some(current),
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(RULES),
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
        ..Default::default()
    };
    let counts = input.append(&mut frame, assets, &refdef, software, None);
    assert_eq!(
        (counts.models, counts.missing_models, counts.rejected),
        (3, 0, 0)
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
            result[item.index as usize] = Some(CapturedImage {
                id,
                first: image.indexed.as_ref().unwrap().mip(0).unwrap().indices()[0],
                size: image.logical_size(),
            });
        }
    }
    result.map(Option::unwrap)
}

fn prepare(host: &mut FrameHost, source: &mut Source) {
    host.register_client_model_assets(Some(&mut source.assets));
}

#[test]
fn received_qw_peers_translate_only_the_registered_player_and_keep_local_skins() {
    let (_fixture, mut host, mut source) = loading();
    peers(
        &mut host,
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
    setting(&mut host, "cl_noskins", "2");
    prepare(&mut host, &mut source);
    assert_eq!(draw(&host, true), translated);

    setting(&mut host, "gl_nocolors", "1");
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
    assert_eq!(draw(&host, true), translated);
}

#[test]
fn missing_qw_skin_uses_base_but_present_malformed_skin_uses_the_model() {
    let (_fixture, mut host, mut source) = loading();
    peers(
        &mut host,
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
    let before = draw(&host, true);
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
    let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
    assert_eq!(state.models.revision(), revision);
    assert_eq!(state.peer(0).unwrap().metadata.user_id, Some(101));
    assert_eq!(state.peer(1).unwrap().metadata.user_id, Some(202));
}
