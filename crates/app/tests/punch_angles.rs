use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{ClientView, FrameHost, FrameResult, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{loopback::Endpoint, primitives::*, sys_events::*};
use qa_network::{commands::packet::Protocol, snapshots::Q2EntityPolicy};
use qa_render::world::{WorldId, WorldPresentationId};
use qa_session::timing::TickRate;
use std::time::Duration;

struct Source {
    time: EventTime,
    view: Option<ClientView>,
    listener: Option<qa_audio::Listener>,
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
    fn wait_time(&mut self, remaining: Duration) -> Duration {
        self.time.0 += remaining.as_nanos() as u64;
        Duration::from_nanos(self.time.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn listeners(&mut self, listeners: &[Option<qa_audio::Listener>; SeatId::COUNT], _: f32) {
        self.listener = listeners[0];
    }
    fn render(
        &mut self,
        _: Option<&mut qa_app::renderer::SceneComposer>,
        views: &[Option<ClientView>; SeatId::COUNT],
        _: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        _: &[Option<qa_app::renderer::hud::HudView<'_>>; SeatId::COUNT],
        _: qa_console::presentation::ConsoleView<'_>,
    ) {
        self.view = views[0];
    }
}

fn fixture(protocol: Protocol, presentation: RuleSetId) -> (FrameHost, Source, ClientId) {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = runtime
        .connect_local(
            SeatId::new(0).unwrap(),
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3([5.0, 20.0, -1.0]),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: presentation,
                client: presentation,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::QuakeWorld,
            },
            protocol,
            0,
        )
        .unwrap();
    if protocol == Protocol::Quake2Repro1038 {
        for endpoint in [Endpoint::Server, Endpoint::Client] {
            runtime
                .network
                .get_mut(client, endpoint)
                .unwrap()
                .channel
                .configure_q2_repro(100, Q2EntityPolicy::default())
                .unwrap();
        }
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    host.local_worlds[0] = Some((WorldId(0), WorldPresentationId(0)));
    (
        host,
        Source {
            time: EventTime::default(),
            view: None,
            listener: None,
        },
        client,
    )
}

fn snapshot(
    host: &mut FrameHost,
    source: &mut Source,
    client: ClientId,
    native_ms: u64,
    queued_ms: u64,
    punch: Vec3,
    position: Vec3,
) {
    let player = &mut host.runtime.server.clients[client.0 as usize].player;
    player.punch_angles = punch;
    player.body.position = position;
    assert!(host.runtime.send_snapshot(
        client,
        EventTime::from_milliseconds(native_ms),
        None,
        source,
        |_, _| None
    ));
    host.queue
        .push(SysEvent {
            time: EventTime::from_milliseconds(queued_ms),
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut FrameResult::default());
    let channel = &host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel;
    host.runtime.snapshots[client.0 as usize]
        .as_mut()
        .unwrap()
        .advance_presentation(
            channel,
            EventTime::from_milliseconds(queued_ms),
            Default::default(),
        )
        .unwrap();
    // Camera feedback fixtures retain their exact accepted view; movement and
    // its angle quantization are tested independently below.
    host.runtime.prediction[0].player.prediction = PredictionPolicy::DISABLED;
}

fn presented(host: &mut FrameHost, source: &mut Source, time: u64) -> Vec3 {
    source.time = EventTime::from_milliseconds(time);
    let client = host.local_clients[0].unwrap();
    let channel = &host
        .runtime
        .network
        .get(client, Endpoint::Server)
        .unwrap()
        .channel;
    let binding = host.runtime.snapshots[client.0 as usize].as_mut().unwrap();
    // Sample the already accepted CLIENT frame without introducing a different
    // SERVER snapshot in the presentation-only frame below.
    binding.submitted(binding.sequence(channel, source.time));
    host.frame(source, true);
    let view = source.view.unwrap();
    assert_eq!(
        source.listener.unwrap().right,
        qa_core::math::angle_vectors(view.angles).right
    );
    assert_eq!(host.client_views()[0].unwrap().angles, view.angles);
    let refdef = qa_app::renderer::view_refdef(
        view,
        qa_render::Viewport {
            width: 320,
            height: 200,
            ..Default::default()
        },
        qa_app::renderer::Kind::Cpu,
    );
    assert_eq!(
        refdef.axes,
        qa_core::math::angle_vectors(view.angles).axes()
    );
    view.angles
}

#[test]
fn queued_q2_and_rerelease_kicks_drive_camera_and_listener_without_changing_player_angles() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let (mut host, mut source, client) = fixture(protocol, protocol.rule_set());
        snapshot(
            &mut host,
            &mut source,
            client,
            100,
            200,
            Vec3([4.0, -2.0, 1.0]),
            Vec3::default(),
        );
        snapshot(
            &mut host,
            &mut source,
            client,
            200,
            200,
            Vec3([-4.0, 6.0, -1.0]),
            Vec3::default(),
        );
        let accepted = host.runtime.prediction[0].player.view_angles;
        assert_eq!(
            presented(&mut host, &mut source, 250),
            accepted + Vec3([0.0, 2.0, 0.0])
        );
        let player = &host.runtime.prediction[0].player;
        assert_eq!(player.view_angles, accepted);
        assert_eq!(player.punch_angles, Vec3([-4.0, 6.0, -1.0]));
        assert_eq!(
            (player.movement_rules, player.trace_rules),
            (RuleSetId::Quake3, RuleSetId::QuakeWorld)
        );
    }
}

#[test]
fn missing_exact_previous_frame_and_native_teleport_disable_q2_kick_interpolation() {
    for (current_ms, position) in [(300, Vec3::default()), (200, Vec3([512.0, 0.0, 0.0]))] {
        let (mut host, mut source, client) = fixture(Protocol::Quake2_34, RuleSetId::Quake2);
        snapshot(
            &mut host,
            &mut source,
            client,
            100,
            current_ms,
            Vec3([4.0, 0.0, 0.0]),
            Vec3::default(),
        );
        snapshot(
            &mut host,
            &mut source,
            client,
            current_ms,
            current_ms,
            Vec3([-4.0, 0.0, 0.0]),
            position,
        );
        let accepted = host.runtime.prediction[0].player.view_angles;
        assert_eq!(
            presented(&mut host, &mut source, current_ms + 50),
            accepted + Vec3([-4.0, 0.0, 0.0])
        );
    }
}

#[test]
fn explicit_presentation_role_controls_kick_independently_of_wire_and_movement() {
    for (presentation, expected) in [
        (RuleSetId::Quake, Vec3([-4.0, 6.0, -1.0])),
        (RuleSetId::Quake3, Vec3::default()),
    ] {
        let (mut host, mut source, client) = fixture(Protocol::Quake2_34, presentation);
        snapshot(
            &mut host,
            &mut source,
            client,
            100,
            200,
            Vec3([4.0, -2.0, 1.0]),
            Vec3::default(),
        );
        snapshot(
            &mut host,
            &mut source,
            client,
            200,
            200,
            Vec3([-4.0, 6.0, -1.0]),
            Vec3::default(),
        );
        let accepted = host.runtime.prediction[0].player.view_angles;
        assert_eq!(presented(&mut host, &mut source, 250), accepted + expected);
        let binding = host.runtime.snapshots[client.0 as usize].as_ref().unwrap();
        assert_eq!(binding.view_source(), Some(presentation));
        assert_eq!(
            binding.weapon_view_angles(accepted, accepted + expected),
            if presentation == RuleSetId::Quake {
                accepted
            } else {
                accepted + expected
            }
        );
    }
}

#[test]
fn netquake_wire_punch_is_direct_and_q3_has_no_player_punch() {
    for protocol in [Protocol::NetQuake15, Protocol::Quake3_68] {
        let (mut host, mut source, client) = fixture(protocol, protocol.rule_set());
        snapshot(
            &mut host,
            &mut source,
            client,
            100,
            100,
            Vec3([2.0, -4.0, 1.0]),
            Vec3::default(),
        );
        let accepted = host.runtime.prediction[0].player.view_angles;
        if protocol == Protocol::Quake3_68 {
            host.runtime.prediction[0].player.punch_angles = Vec3([2.0, -4.0, 1.0]);
        }
        let punch = if protocol == Protocol::NetQuake15 {
            Vec3([2.0, -4.0, 1.0])
        } else {
            Vec3::default()
        };
        assert_eq!(presented(&mut host, &mut source, 100), accepted + punch);
    }
}

#[test]
fn waiting_client_collision_honors_angle_prediction_without_changing_position() {
    for enabled in [false, true] {
        let (mut host, mut source, client) = fixture(Protocol::NetQuake15, RuleSetId::Quake);
        snapshot(
            &mut host,
            &mut source,
            client,
            100,
            100,
            Vec3::default(),
            Vec3([40.0, 50.0, 60.0]),
        );
        let accepted = host.runtime.prediction[0].player.view_angles;
        let position = host.runtime.prediction[0].player.body.position;
        host.runtime.prediction[0].player.prediction.angles = enabled;
        let angles = presented(&mut host, &mut source, 100);
        let expected = if enabled {
            // Q3's signed16 angle units are 910, 3640 and -182 here.
            Vec3([4.998779296875, 19.9951171875, -0.999755859375])
        } else {
            accepted
        };
        assert_eq!(angles, expected);
        assert_eq!(host.runtime.prediction[0].player.view_angles, expected);
        assert_eq!(host.runtime.prediction[0].player.body.position, position);
        assert!(matches!(
            host.runtime.client_collision[client.0 as usize]
                .as_mut()
                .unwrap()
                .trace(&host.runtime.geometry),
            Err(qa_app::client_collision::ClientCollisionError::Waiting)
        ));
    }
}

#[test]
fn rerelease_kick_switch_does_not_clear_accepted_punch_or_native_view_angles() {
    let (mut host, mut source, client) =
        fixture(Protocol::Quake2Repro1038, RuleSetId::Quake2Rerelease);
    snapshot(
        &mut host,
        &mut source,
        client,
        100,
        200,
        Vec3([4.0, -2.0, 1.0]),
        Vec3::default(),
    );
    snapshot(
        &mut host,
        &mut source,
        client,
        200,
        200,
        Vec3([-4.0, 6.0, -1.0]),
        Vec3::default(),
    );
    let view = host
        .console
        .cvars
        .bind(
            "cl_kickangles",
            Context {
                source: RuleSetId::Quake2Rerelease,
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(view, "0").unwrap();
    let accepted = host.runtime.prediction[0].player.view_angles;
    assert_eq!(presented(&mut host, &mut source, 250), accepted);
    assert_eq!(
        host.runtime.prediction[0].player.punch_angles,
        Vec3([-4.0, 6.0, -1.0])
    );
}
