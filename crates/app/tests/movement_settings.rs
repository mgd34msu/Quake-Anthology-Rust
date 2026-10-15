#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod support;

use qa_app::{
    Runtime, WorldCollision,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{Bounds, ClientId, RuleSetId, UserCmd, Vec3},
    sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_network::commands::packet::Protocol;
use qa_session::timing::TickRate;
use qa_world::collision::{brushes::BrushTree, surfaces::SurfaceTable};
use std::time::Duration;

fn host() -> (FrameHost, ClientId) {
    let mut runtime = Runtime::load(1, []).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            Vec::new(),
            SurfaceTable::flags(vec![]),
            BrushTree::direct(0).unwrap(),
            vec![Bounds {
                mins: Vec3([-4096.0; 3]),
                maxs: Vec3([4096.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    // Presentation and packet format do not choose movement cvar defaults.
    let id = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3([0.0, 0.0, 128.0]),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake2,
                client: RuleSetId::Quake2,
                movement: RuleSetId::Quake,
                trace: RuleSetId::Quake3,
            },
            Protocol::QuakeWorld28,
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
    let view = host
        .console
        .cvars
        .bind(
            "sv_gravity",
            Context {
                source: RuleSetId::Quake,
                ..Context::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(view, "100").unwrap();
    (host, id)
}

fn check_velocity(host: &FrameHost, id: ClientId, expected: f32, base: Option<f32>) {
    let player = &host.runtime.server.clients[id.0 as usize].player;
    assert!(
        (player.body.velocity.0[2] - expected).abs() < 0.00001,
        "{:?}",
        player.body.velocity
    );
    assert_eq!(
        player.movement.tuning.gravity,
        base.map(qa_core::primitives::MovementMagnitude::Base)
    );
    assert_eq!(player.trace_rules, RuleSetId::Quake3);
}

#[test]
fn queued_authoritative_commands_resolve_aliases_and_do_not_keep_removed_overrides() {
    let (mut host, id) = host();
    let command = UserCmd {
        duration_ms: 16,
        ..Default::default()
    };
    assert!(
        host.runtime
            .send_local_command(id, &command, EventTime::from_milliseconds(16))
    );
    host.drain(&mut Default::default());
    check_velocity(&host, id, -1.6, None);
    let gravity = host.console.cvars.find("g_gravity").unwrap();
    host.console.cvars.reset(gravity);
    host.runtime.server.clients[id.0 as usize]
        .player
        .body
        .velocity = Vec3::default();
    assert!(
        host.runtime
            .send_local_command(id, &command, EventTime::from_milliseconds(32))
    );
    host.drain(&mut Default::default());
    check_velocity(&host, id, -12.8, None);
}

#[test]
fn native_pmove_service_uses_the_same_resolver_and_retains_module_gravity() {
    let (mut host, id) = host();
    host.runtime.server.clients[id.0 as usize]
        .player
        .movement
        .tuning
        .gravity = Some(600.0.into());
    let mut storage =
        qa_compat::services::ServiceStorage::load(&[], 0, &host.console.cvars).unwrap();
    let mut scratch = host.runtime.geometry.scratch();
    host.runtime.server.clients[id.0 as usize].command = UserCmd {
        duration_ms: 16,
        ..Default::default()
    };
    host.runtime
        .engine_services(&mut host.console, &mut storage, &mut scratch)
        .pmove(id, None, Default::default())
        .unwrap();
    check_velocity(&host, id, -9.6, Some(600.0));
    let gravity = host.console.cvars.find("g_gravity").unwrap();
    host.console.cvars.reset(gravity);
    host.runtime.server.clients[id.0 as usize]
        .player
        .body
        .velocity = Vec3::default();
    host.runtime
        .engine_services(&mut host.console, &mut storage, &mut scratch)
        .pmove(id, None, Default::default())
        .unwrap();
    check_velocity(&host, id, -9.6, Some(600.0));
}

#[derive(Default)]
struct Source(u64);
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(self.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(self.0),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        self.0 += 1;
        Duration::from_millis(self.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

#[test]
fn client_prediction_resolves_the_movement_role_settings_after_snapshot_application() {
    let (mut host, id) = host();
    host.local_clients[0] = Some(id);
    // The immutable geometry may be shared, but prediction must select it from
    // the accepted CLIENT configuration rather than the SERVER world binding.
    let geometry = host.runtime.collision.as_ref().unwrap().geometry;
    let mut assets = qa_render::Assets::load().unwrap();
    let world = support::world(&mut assets, 0.0);
    let presentation =
        qa_render::WorldPresentationId(assets.world(world).unwrap().presentations().first);
    let profile = qa_app::model_assets::ModelRequest::native("", RuleSetId::Quake2, None).profile();
    let mut builder = qa_app::model_assets::ModelCatalogBuilder::new(
        assets,
        [profile],
        [world],
        qa_app::model_assets::ModelLimits::default(),
    )
    .unwrap();
    builder
        .bind_loaded_world(
            "maps/prediction.bsp",
            world,
            Some(geometry),
            host.runtime.geometry.model_bounds(geometry, 0).unwrap(),
        )
        .unwrap();
    let (_, catalog) = builder.finish();
    let resources = &mut host.runtime.client_resources[id.0 as usize]
        .as_mut()
        .unwrap()
        .state;
    resources
        .configs
        .set(
            resources.layout.resources[0].get(1).unwrap(),
            b"maps/prediction.bsp",
        )
        .unwrap();
    assert_eq!(
        resources.models.bind_all(
            &resources.configs,
            &catalog,
            profile,
            Some((world, presentation)),
        ),
        0
    );
    assert_eq!(resources.models.collision_world(), Some((geometry, 0)));
    host.runtime.models = Some(std::sync::Arc::new(catalog));
    let mut source = Source::default();
    host.frame(&mut source, true);
    source.0 = 16;
    host.frame(&mut source, true);
    let prediction = &host.runtime.prediction[0].player;
    assert!(
        (prediction.body.velocity.0[2] + 1.6).abs() < 0.00001,
        "{:?}",
        prediction.body.velocity
    );
    assert!(
        host.runtime.snapshots[id.0 as usize]
            .as_ref()
            .unwrap()
            .applied
            > 0
    );
    assert_eq!(prediction.movement.tuning.gravity, None);
    assert_eq!(prediction.trace_rules, RuleSetId::Quake3);
    assert!(
        host.runtime.client_collision[id.0 as usize]
            .as_mut()
            .unwrap()
            .trace(&host.runtime.geometry)
            .is_ok()
    );
}
