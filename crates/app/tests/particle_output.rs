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
    particles::register_point_material,
    surface_cache::PaletteLighting,
};
use qa_session::{clients::Connection, timing::TickRate};
use std::{sync::Arc, time::Duration};

const MODULE: ModuleId = ModuleId(7);
const REMOTE_PORT: u16 = 23999;

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
        let material = register_point_material(&mut assets, "particles/queued-native").unwrap();
        let catalog = Arc::clone(&runtime.catalog.registry.effects);
        let mut bindings = vec![None; catalog.len() + 1];
        for (index, definition) in catalog.definitions().iter().enumerate() {
            if definition.source == RuleSetId::Quake {
                bindings[index + 1] = Some(ParticleBinding {
                    palette,
                    material,
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
    fn effect(&mut self, event: EffectEvent) -> bool {
        // Acceptance is the production particle arena, not this observation.
        let accepted = self.effects.accept(event);
        if accepted {
            self.last_effect = Some(event);
        }
        accepted
    }
    fn render(
        &mut self,
        _: Option<&mut qa_app::renderer::SceneComposer>,
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
            assert!(frame.render_scene(Refdef::default(), &[], &self.assets));
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

fn advance_provider(host: &mut FrameHost, _: qa_session::timing::Tick) {
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
    assert_eq!(sent.output.native_packets, 1);
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
        .bind_server_connection(
            remote,
            NativeConnection {
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
            1,
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
