use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource, Provider},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{events::FrameEvent, loopback::Endpoint, primitives::*, sys_events::*};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as NativeConnection, Route},
};
use qa_render::{
    Assets, Command, FrontEnd, Limits, ParticlePresentation, Refdef,
    effects::{EffectClient, Effects, ParticleBinding},
    particles::register_point_shader,
    surface_cache::PaletteLighting,
};
use qa_session::{clients::Connection, timing::TickRate};
use std::{sync::Arc, time::Duration};

const MODULE: ModuleId = ModuleId(7);
const REMOTE_PORT: u16 = 23999;

#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod asset_support;

#[path = "support/client_asset_frames.rs"]
mod asset_frames;

struct Source {
    time: EventTime,
    effects: Effects,
    assets: Assets,
    front: FrontEnd,
    clients: [Option<EffectClient>; SeatId::COUNT],
    last_effect: Option<EffectEvent>,
    appended: u32,
    remote_attempts: usize,
    gravity: f32,
}
impl Source {
    fn load(runtime: &Runtime) -> Self {
        let mut assets = Assets::load().unwrap();
        let colors: Vec<u8> = (0..256).flat_map(|n| [n as u8; 3]).collect();
        let palette = assets
            .register_palette(
                PaletteLighting::load(&colors, &vec![0; 64 * 256], None, 256).unwrap(),
            )
            .unwrap();
        let material = register_point_shader(&mut assets, "particles/queued-native").unwrap();
        let catalog = Arc::clone(&runtime.catalog.registry.effects);
        let mut bindings = vec![None; catalog.len() + 1];
        for (index, definition) in catalog.definitions().iter().enumerate() {
            if definition.source == RuleSetId::Quake {
                bindings[index + 1] = Some(ParticleBinding {
                    palette,
                    shader: material,
                    presentation: ParticlePresentation::Square { pixel_aspect: 1.0 },
                });
            }
        }
        Self {
            time: EventTime::from_milliseconds(1000),
            effects: Effects::load(catalog, 64, runtime.server.clients.len(), &bindings).unwrap(),
            assets,
            front: FrontEnd::load(Limits {
                particles: 64,
                ..Limits::default()
            })
            .unwrap(),
            clients: [None; SeatId::COUNT],
            last_effect: None,
            appended: 0,
            remote_attempts: 0,
            gravity: 0.0,
        }
    }
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.time,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped fixture");
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn effect_clients(
        &mut self,
        clients: &[Option<EffectClient>; SeatId::COUNT],
        time: EventTime,
        gravity: f32,
    ) {
        self.clients = *clients;
        self.gravity = gravity;
        self.effects.synchronize(clients);
        for client in clients.iter().flatten() {
            assert!(
                self.effects
                    .begin_frame(client.client, client.lifetime, time, gravity)
            );
        }
    }
    fn effect(&mut self, _palette: Option<qa_render::PaletteId>, event: EffectEvent) -> bool {
        // Acceptance is the production particle arena, not this observation.
        let accepted = self.effects.accept(event);
        if accepted {
            self.last_effect = Some(event);
        }
        accepted
    }
    fn reset_presentation(&mut self, client: ClientId, lifetime: EntityId) -> bool {
        self.effects.reset_client(client, lifetime);
        true
    }
    fn render(
        &mut self,
        _: Option<&mut qa_app::renderer::SceneComposer>,
        _: Option<qa_app::model_assets::ModelView<'_>>,
        _native: &dyn qa_render::shader::reference::NativeShaders,
        _: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
        _: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        _: &[Option<qa_app::renderer::hud::HudView<'_>>; SeatId::COUNT],
        _: qa_console::presentation::ConsoleView<'_>,
    ) {
        let mut frame = self.front.begin_frame([0, 0, 0, 255]).unwrap();
        self.appended = 0;
        for client in self.clients.iter().flatten() {
            frame.clear_scene();
            self.appended += self
                .effects
                .append(&mut frame, client.client, client.lifetime);
            assert!(frame.render_scene(
                Refdef::default(),
                &[],
                &self.assets,
                qa_render::shader::reference::ShaderView::direct()
            ));
            // The renderer reads copied scene records; simulation owns its SoA.
            self.effects
                .finish_frame(client.client, client.lifetime, self.time, self.gravity);
        }
        let packet = frame.finish();
        let stored: usize = packet
            .commands()
            .iter()
            .filter_map(|command| match command {
                Command::View(view) => Some(packet.particles(view.scene.particles).len()),
                _ => None,
            })
            .sum();
        assert_eq!(stored, self.appended as usize);
        assert_eq!(packet.rejected, 0);
        assert!(self.front.recycle(packet).is_ok());
    }
    fn send_packet(&mut self, _: u16, peer: std::net::SocketAddr, _: &[u8]) -> bool {
        assert_eq!(peer.port(), REMOTE_PORT);
        self.remote_attempts += 1;
        false
    }
}

fn connect_local(runtime: &mut Runtime) -> ClientId {
    runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(
                Some(RuleSetId::Quake),
                None,
                Some(RuleSetId::Quake3),
                Some(RuleSetId::Quake2),
                None,
            )
            .unwrap(),
            Protocol::NetQuake15,
            0,
        )
        .unwrap()
}

fn advance_provider(
    host: &mut FrameHost,
    _: qa_session::timing::Tick,
    _driver: &mut qa_app::modules::Driver<'_>,
) {
    for client in host.runtime.server.clients.iter_mut() {
        if client.connection.is_some() {
            client.player.health += 1;
        }
    }
}

fn host(max_clients: usize) -> (FrameHost, ClientId, Source) {
    let mut runtime = Runtime::load(max_clients, []).unwrap();
    let client = connect_local(&mut runtime);
    let source = Source::load(&runtime);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(20).unwrap(),
        vec![Provider {
            module: MODULE,
            rate: TickRate::fixed(20).unwrap(),
            frame: advance_provider,
            output: None,
        }],
    )
    .unwrap();
    host.local_clients[SeatId::FIRST.index()] = Some(client);
    (host, client, source)
}

fn publish(host: &mut FrameHost, target: NativeTarget) -> (u64, EffectEvent) {
    let event = host
        .runtime
        .catalog
        .registry
        .effects
        .native_particles(RuleSetId::Quake)
        .unwrap()
        .event(Vec3([64.125, 0.0, 0.0]), Vec3([1.0, -2.0, 0.5]), 73, 1);
    let sequence = host
        .runtime
        .server
        .events
        .push_delivered(
            NativeDelivery {
                module: MODULE,
                source: RuleSetId::Quake,
                target,
                origin: event.position,
                reliable: false,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: false,
            },
            FrameEvent::Effect(event),
        )
        .unwrap();
    (sequence, event)
}

#[test]
fn server_particle_reaches_real_client_arena_once_through_queued_native_packet() {
    let (mut host, client, mut source) = host(1);
    let (_, server_event) = publish(&mut host, NativeTarget::Client(client));
    let lifetime = host.runtime.server.clients[client.0 as usize].entity;
    let sent = host.frame(&mut source, true);
    assert!(sent.output.native_packets > 0);
    assert_eq!(sent.output.effects, 0);
    assert_eq!(source.effects.stats.accepted_events, 0);
    assert!(source.effects.is_empty());
    let received = host.frame(&mut source, true);
    assert_eq!(received.output.effects, 1);
    assert_eq!(received.output.unhandled_effects, 0);
    assert_eq!(source.effects.stats.accepted_events, 1);
    assert_eq!(source.effects.stats.spawned, 1);
    assert_eq!(source.appended, 1);
    let event = source.last_effect.unwrap();
    assert_eq!(
        (event.client, event.client_lifetime),
        (Some(client), Some(lifetime))
    );
    assert_eq!(event.time, source.time);
    assert_eq!(
        (
            event.effect,
            event.position,
            event.direction,
            event.color,
            event.count
        ),
        (
            server_event.effect,
            server_event.position,
            server_event.direction,
            73,
            1
        )
    );
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .movement_rules,
        RuleSetId::Quake3
    );
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .trace_rules,
        RuleSetId::Quake2
    );
    assert_eq!(
        host.runtime
            .catalog
            .registry
            .effects
            .definition(event.effect)
            .unwrap()
            .source,
        RuleSetId::Quake
    );
    let again = host.frame(&mut source, true);
    assert_eq!(again.output.effects, 0);
    assert_eq!(source.effects.stats.accepted_events, 1);
    assert_eq!(source.effects.stats.spawned, 1);
    assert_eq!(host.runtime.network.command_errors, 0);
}

#[test]
fn rejected_remote_particle_keeps_local_receipt_and_provider_ticks_running() {
    let (mut host, client, mut source) = host(2);
    let remote = host
        .runtime
        .server
        .connect(Connection::Remote, MODULE, None)
        .unwrap();
    host.runtime
        .bind_connection(
            remote,
            NativeConnection {
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: 0,
                    peer: Peer::Socket(([127, 0, 0, 1], REMOTE_PORT).into()),
                },
                channel: Channel::load(Protocol::NetQuake15.channel(), Endpoint::Server, 8192, 16)
                    .unwrap(),
                output: None,
                pending_output: None,
                commands: Some(Commands::load(Protocol::NetQuake15, 8192)),
            },
            qa_app::snapshots::NativePlayerBinding::Supplied(1),
            None,
        )
        .unwrap();
    let remote_output = host.runtime.server.clients[remote.0 as usize]
        .output
        .unwrap();
    let local_output = host.runtime.server.clients[client.0 as usize]
        .output
        .unwrap();
    let (sequence, _) = publish(&mut host, NativeTarget::All);
    let first = host.frame(&mut source, true);
    assert!(first.output.native_blocked > 0);
    assert!(
        host.runtime
            .server
            .events
            .retained_for(remote_output, sequence)
    );
    assert!(
        !host
            .runtime
            .server
            .events
            .retained_for(local_output, sequence)
    );
    source.time = EventTime::from_milliseconds(1020);
    let health = host.runtime.server.clients[client.0 as usize].player.health;
    let next = host.frame(&mut source, true);
    assert!(next.server_ticks > 0);
    assert!(host.runtime.server.clients[client.0 as usize].player.health > health);
    assert_eq!(source.effects.stats.accepted_events, 1);
    assert_eq!(source.effects.stats.spawned, 1);
    assert!(source.remote_attempts > 1);
    assert!(
        host.runtime
            .server
            .events
            .retained_for(remote_output, sequence)
    );
    assert_eq!(
        host.runtime.server.clients[remote.0 as usize].connection,
        Some(Connection::Remote)
    );
}

#[test]
fn reconnect_drops_queued_old_packet_and_retires_old_particle_lifetime() {
    let (mut host, client, mut source) = host(1);
    publish(&mut host, NativeTarget::Client(client));
    host.frame(&mut source, true);
    host.frame(&mut source, true);
    assert_eq!(source.effects.stats.accepted_events, 1);
    let old_lifetime = host.runtime.server.clients[client.0 as usize].entity;
    let old_event = source.last_effect.unwrap();
    publish(&mut host, NativeTarget::Client(client));
    host.frame(&mut source, true);
    assert!(host.runtime.disconnect_client(client));
    // NQ owns audio resources too: retire the old CLIENT through the existing
    // Presentation consumer before that same bounded client slot is reusable.
    host.frame(&mut source, true);
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .audio_retirement
            .is_none()
    );
    let rebound = connect_local(&mut host.runtime);
    assert_eq!(rebound, client);
    let new_lifetime = host.runtime.server.clients[client.0 as usize].entity;
    assert_ne!(new_lifetime, old_lifetime);
    host.local_clients[SeatId::FIRST.index()] = Some(rebound);
    host.frame(&mut source, true);
    assert_eq!(source.effects.stats.accepted_events, 1);
    assert_eq!(source.effects.stats.retired, 1);
    assert!(source.effects.is_empty());
    host.runtime
        .server
        .events
        .push_to(
            qa_core::events::OutputTarget::Presentation,
            FrameEvent::Effect(old_event),
        )
        .unwrap();
    let stale = host.frame(&mut source, true);
    assert_eq!(
        (stale.output.effects, stale.output.unhandled_effects),
        (1, 1)
    );
    assert_eq!(source.effects.stats.accepted_events, 1);
    assert!(source.effects.is_empty());
    publish(&mut host, NativeTarget::Client(rebound));
    host.frame(&mut source, true);
    host.frame(&mut source, true);
    assert_eq!(source.effects.stats.accepted_events, 2);
    assert_eq!(
        source.last_effect.unwrap().client_lifetime,
        Some(new_lifetime)
    );
    assert_eq!(host.runtime.network.command_errors, 0);
}

struct EffectReceipt {
    palette: Option<qa_render::PaletteId>,
    accepted: bool,
    client: Option<ClientId>,
    lifetime: Option<EntityId>,
}

struct RendererSource {
    time: EventTime,
    renderer: qa_app::renderer::Renderer,
    clients: [Option<EffectClient>; SeatId::COUNT],
    delivered: Vec<EffectReceipt>,
}
impl RendererSource {
    fn load(host: &mut FrameHost) -> Self {
        use qa_app::{
            model_assets::ModelAssets,
            renderer::{Context, Renderer, console::ConsoleAssets},
        };
        let mut owner = ModelAssets::new(
            &host.runtime.vfs,
            Assets::load().unwrap(),
            [],
            [],
            Default::default(),
        )
        .unwrap();
        let console = ConsoleAssets::load(
            &host.runtime.vfs,
            &mut owner,
            RuleSetId::Quake3,
            qa_render::material::resources::ImageSettings::native(RuleSetId::Quake3),
            qa_render::material::ShaderCompilePolicy::default(),
            160,
            120,
        )
        .unwrap();
        let catalog = Arc::clone(&host.runtime.catalog.registry.effects);
        let bindings = vec![None; catalog.len() + 1];
        let effects =
            Effects::load(catalog, 64, host.runtime.server.clients.len(), &bindings).unwrap();
        let configuration = Arc::new(qa_core::presentation::RendererConfiguration {
            display: qa_core::presentation::DisplayConfiguration {
                width: 160,
                height: 120,
                ..Default::default()
            },
            ..Default::default()
        });
        host.runtime.model_assets = Some(owner);
        let huds = std::array::from_fn(|_| None);
        let model_choices = host
            .runtime
            .model_assets
            .as_ref()
            .unwrap()
            .catalog()
            .max_choice_nodes();
        let (assets, staged) = host.stage_engine_graphics(&console, &huds).unwrap();
        let (renderer, scene) = Renderer::load(
            Context::Cpu(configuration),
            160,
            120,
            assets,
            vec![],
            Some(qa_render::cpu::RasterBands::One),
            console,
            huds,
            effects,
            model_choices,
        )
        .unwrap();
        staged.commit();
        host.publish_model_admission(renderer.admitted_assets().unwrap())
            .unwrap();
        host.runtime
            .model_assets
            .as_mut()
            .unwrap()
            .end_initial_registration();
        host.runtime.scene = Some(scene);
        Self {
            time: EventTime::from_milliseconds(1000),
            renderer,
            clients: [None; SeatId::COUNT],
            delivered: Vec::new(),
        }
    }
}
impl FrameSource for RendererSource {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.time,
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
    fn effect_clients(
        &mut self,
        clients: &[Option<EffectClient>; SeatId::COUNT],
        time: EventTime,
        gravity: f32,
    ) {
        self.clients = *clients;
        self.renderer.set_effect_clients(*clients, time, gravity);
    }
    fn effect(&mut self, palette: Option<qa_render::PaletteId>, event: EffectEvent) -> bool {
        let accepted = self.renderer.effect(palette, event);
        self.delivered.push(EffectReceipt {
            palette,
            accepted,
            client: event.client,
            lifetime: event.client_lifetime,
        });
        accepted
    }
    fn reset_presentation(&mut self, client: ClientId, lifetime: EntityId) -> bool {
        self.renderer.reset_client_effects(client, lifetime);
        true
    }
    fn render(
        &mut self,
        scene: Option<&mut qa_app::renderer::SceneComposer>,
        resources: Option<qa_app::model_assets::ModelView<'_>>,
        native: &dyn qa_render::shader::reference::NativeShaders,
        views: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
        scenes: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        hud: &[Option<qa_app::renderer::hud::HudView<'_>>; SeatId::COUNT],
        console: qa_console::presentation::ConsoleView<'_>,
    ) {
        let scene = scene.unwrap();
        let resources = resources.unwrap();
        // Exercise the production native-scene particle annotation and the
        // real CPU Renderer. No map or second particle simulation is needed.
        assert!(
            scene
                .frame_mut()
                .unwrap()
                .for_client(self.clients[0], |frame| frame.render_scene(
                    Refdef {
                        viewport: qa_render::Viewport {
                            width: 160,
                            height: 120,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    &[],
                    resources.assets,
                    qa_render::shader::reference::ShaderView::direct()
                ))
        );
        self.renderer
            .frame(scene, resources, native, views, scenes, hud, console);
    }
}

#[test]
fn queued_particles_use_only_the_late_backend_admitted_palette_in_the_real_renderer() {
    use qa_app::renderer::AssetPreparer;
    let (mut host, client, _) = host(1);
    let mut source = RendererSource::load(&mut host);
    let retained = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .published_palette(RuleSetId::Quake, None),
        None
    );
    publish(&mut host, NativeTarget::Client(client));
    host.frame(&mut source, true);
    let missing = host.frame(&mut source, true);
    assert_eq!(
        (missing.output.effects, missing.output.unhandled_effects),
        (1, 1)
    );
    assert_eq!(source.delivered.len(), 1);
    assert_eq!(
        (source.delivered[0].palette, source.delivered[0].accepted),
        (None, false)
    );

    let rgb: Vec<_> = (0..=255u8).flat_map(|value| [value; 3]).collect();
    let mut shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    shades.push(32);
    let fixture =
        asset_support::Fixture::new(&[("gfx/palette.lmp", rgb), ("gfx/colormap.lmp", shades)]);
    for mount in fixture.vfs.mounts() {
        host.runtime
            .vfs
            .mount_directory(&mount.path, mount.priority)
            .unwrap();
    }
    let owner = host.runtime.model_assets.as_mut().unwrap();
    let rejected = owner.prepare_profile(
        &host.runtime.vfs,
        RuleSetId::Quake,
        RuleSetId::Quake3,
        qa_render::material::resources::ImageSettings::native(RuleSetId::Quake3),
        qa_render::material::MaterialFinishPolicy::default(),
        None,
        |assets| {
            assert!(assets.counts().palettes > retained.palettes);
            source.renderer.prepare_assets(
                assets,
                Some(Limits {
                    surfaces: 0,
                    ..Default::default()
                }),
            )
        },
    );
    assert!(rejected.is_err());
    assert_eq!(owner.assets().counts(), retained);
    assert_eq!(owner.published_palette(RuleSetId::Quake, None), None);
    assert_eq!(source.renderer.admitted_assets(), Some(retained));
    publish(&mut host, NativeTarget::Client(client));
    host.frame(&mut source, true);
    let rejected = host.frame(&mut source, true);
    assert_eq!(
        (rejected.output.effects, rejected.output.unhandled_effects),
        (1, 1)
    );
    assert_eq!(
        (source.delivered[1].palette, source.delivered[1].accepted),
        (None, false)
    );

    let owner = host.runtime.model_assets.as_mut().unwrap();
    owner
        .prepare_profile(
            &host.runtime.vfs,
            RuleSetId::Quake,
            RuleSetId::Quake3,
            qa_render::material::resources::ImageSettings::native(RuleSetId::Quake3),
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| source.renderer.prepare_assets(assets, None),
        )
        .unwrap();
    let palette = owner.published_palette(RuleSetId::Quake, None).unwrap();
    assert_eq!(
        source.renderer.admitted_assets(),
        Some(owner.assets().counts())
    );
    host.runtime.vfs.take_lookup_count();
    publish(&mut host, NativeTarget::Client(client));
    host.frame(&mut source, true);
    let admitted = host.frame(&mut source, true);
    assert_eq!(
        (admitted.output.effects, admitted.output.unhandled_effects),
        (1, 0)
    );
    assert_eq!(
        (source.delivered[2].palette, source.delivered[2].accepted),
        (Some(palette), true)
    );
    assert_eq!(source.delivered[2].client, Some(client));
    assert_eq!(
        source.delivered[2].lifetime,
        Some(host.runtime.server.clients[client.0 as usize].entity)
    );
    assert_eq!(source.renderer.sample.stats.rejected, 0);
    assert!(source.renderer.sample.stats.views > 0);
    let again = host.frame(&mut source, true);
    assert_eq!(again.output.effects, 0);
    assert_eq!(source.delivered.len(), 3);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
}

#[test]
fn queued_serverinfo_resets_existing_particles_before_new_level_delivery() {
    let (mut host, client, mut source) = host(1);
    publish(&mut host, NativeTarget::Client(client));
    host.frame(&mut source, true);
    host.frame(&mut source, true);
    assert_eq!(source.effects.stats.accepted_events, 1);
    assert_eq!(source.effects.len(), 1);
    // Complete the ordinary native ACK for the initial reliable peer info.
    host.frame(&mut source, true);
    let lifetime = host.runtime.server.clients[client.0 as usize].entity;
    // Native svc_serverinfo: protocol15, one client, game type0, one map,
    // then the empty model and sound list terminators.
    let mut header = vec![11];
    header.extend_from_slice(&15i32.to_le_bytes());
    header.extend_from_slice(&[1, 0]);
    header.extend_from_slice(b"new level\0maps/next.bsp\0\0\0");
    asset_frames::queue(&mut host, [header]);
    let reset = host.frame(&mut source, true);
    assert_eq!(reset.output.presentation_resets, 1);
    assert_eq!(source.effects.stats.retired, 1);
    assert!(source.effects.is_empty());
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].entity,
        lifetime
    );
    publish(&mut host, NativeTarget::Client(client));
    host.frame(&mut source, true);
    let fresh = host.frame(&mut source, true);
    assert_eq!(
        (fresh.output.effects, fresh.output.unhandled_effects),
        (1, 0)
    );
    assert_eq!(source.effects.stats.accepted_events, 2);
    assert_eq!(source.effects.len(), 1);
    assert_eq!(source.last_effect.unwrap().client_lifetime, Some(lifetime));
}
