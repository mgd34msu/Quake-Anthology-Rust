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
        |_, _| None,
        |_, _, _, _| None
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
    assert_eq!(source.listener.unwrap().origin, view.origin);
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
                .trace(&host.runtime.geometry, qa_core::primitives::PatchClip::All),
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

#[test]
fn queued_native_camera_interpolates_client_offset_fov_and_blends_without_server_leakage() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let (mut host, mut source, client) = fixture(protocol, protocol.rule_set());
        for (time, offset, fov, blend) in [
            (100, 22., 90., [0., 0., 1., 0.2]),
            (200, -2., 110., [1., 0., 0., 0.6]),
        ] {
            let view = &mut host.runtime.server.clients[client.0 as usize].player.view;
            view.accept_offset(
                Vec3([0., 0., offset]),
                (protocol == Protocol::Quake2Repro1038).then_some(0.),
            );
            view.fov_x = Some(fov);
            view.screen_blend = blend;
            view.damage_blend = if protocol == Protocol::Quake2Repro1038 {
                blend
            } else {
                [0.; 4]
            };
            view.flags = PlayerViewFlags::INFRARED;
            snapshot(
                &mut host,
                &mut source,
                client,
                time,
                200,
                Vec3::default(),
                Vec3::default(),
            );
        }
        let view = &mut host.runtime.server.clients[client.0 as usize].player.view;
        view.accept_offset(Vec3([900.; 3]), None);
        view.fov_x = Some(1.);
        view.screen_blend = [0., 1., 0., 1.];
        view.damage_blend = [0., 1., 0., 1.];
        presented(&mut host, &mut source, 250);
        let view = source.view.unwrap();
        assert_eq!(view.origin, Vec3([0., 0., 10.]));
        assert_eq!(view.fov_x, 100.);
        assert!(view.flags.contains(PlayerViewFlags::INFRARED));
        assert_eq!(
            (
                host.runtime.prediction[0].player.movement_rules,
                host.runtime.prediction[0].player.trace_rules
            ),
            (RuleSetId::Quake3, RuleSetId::QuakeWorld)
        );
        let refdef = qa_app::renderer::view_refdef(
            view,
            qa_render::Viewport {
                width: 320,
                height: 200,
                ..Default::default()
            },
            qa_app::renderer::Kind::Cpu,
        );
        assert_eq!(refdef.origin, view.origin);
        assert_eq!(refdef.blend, view.screen_blend);
        assert_eq!(refdef.damage_blend, view.damage_blend);
        assert!(view.screen_blend[1] == 0. && view.damage_blend[1] == 0.);
    }
}

#[test]
fn selected_view_entity_teleport_events_cut_camera_pairs_without_resetting_rr_height() {
    for split_height in [false, true] {
        for (selected_event, event) in [(false, 6), (true, 6), (true, 7)] {
            let (mut host, mut source, client) =
                fixture(Protocol::Quake2Repro1038, RuleSetId::Quake2Rerelease);
            let self_entity = host.runtime.server.clients[client.0 as usize].entity;
            let decoy = host
                .runtime
                .server
                .entities
                .allocate(
                    ThinkTime::Seconds(0.0),
                    ModuleId(0),
                    qa_world::entities::AllocationPolicy::EDICT,
                )
                .unwrap()
                .id;
            for entity in [self_entity, decoy] {
                host.runtime.server.entities.columns.appearance[entity.slot as usize].source =
                    RuleSetId::Quake2Rerelease;
            }
            for (time, offset, height, fov) in [
                (100, 0.0, if split_height { 22.0 } else { 0.0 }, 90.0),
                (200, 10.0, if split_height { 12.0 } else { 0.0 }, 110.0),
            ] {
                let player = &mut host.runtime.server.clients[client.0 as usize].player;
                player
                    .view
                    .accept_offset(Vec3([0.0, 0.0, offset]), Some(height));
                player.view.fov_x = Some(fov);
                player.view.flags = PlayerViewFlags::INFRARED;
                if time == 200 {
                    let entity = if selected_event { self_entity } else { decoy };
                    host.runtime.server.entities.columns.appearance[entity.slot as usize]
                        .native
                        .event = event;
                }
                snapshot(
                    &mut host,
                    &mut source,
                    client,
                    time,
                    200,
                    Vec3::default(),
                    Vec3::default(),
                );
            }
            // Both packets retain identical position, flags and POV. Only the
            // event on the selected native entity rejects their camera pair.
            presented(&mut host, &mut source, 250);
            let view = source.view.unwrap();
            let offset = if selected_event { 10.0 } else { 5.0 };
            let height = if split_height { 17.0 } else { 0.0 };
            assert_eq!(
                view.origin,
                Vec3([0.0, 0.0, offset + height]),
                "selected={selected_event} event={event} split={split_height}"
            );
            assert_eq!(view.fov_x, if selected_event { 110.0 } else { 100.0 });
            assert_eq!(view.flags, PlayerViewFlags::INFRARED);
        }
    }
}

fn run_client_frames(host: &mut FrameHost, source: &mut Source, count: u64) {
    let maxfps = host
        .console
        .cvars
        .bind("com_maxfps", Context::default())
        .unwrap();
    host.console.cvars.force_write(maxfps, "0").unwrap();
    for _ in 0..count {
        source.time = EventTime::from_milliseconds(source.time.milliseconds() + 100);
        host.frame(source, true);
    }
}

#[test]
fn q2_userinfo_packets_apply_each_builtin_fov_policy_and_console_updates_reach_client() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let (mut host, mut source, client) = fixture(protocol, protocol.rule_set());
        // Prime ordinary userinfo and its revision cache before injecting the
        // same protocol's accepted remote-style userinfo records.
        run_client_frames(&mut host, &mut source, 3);
        for (text, expected) in [
            (
                &b"\\name\\player"[..],
                if protocol == Protocol::Quake2_34 {
                    90.
                } else {
                    1.
                },
            ),
            (
                &b"\\name\\player\\fov\\0"[..],
                if protocol == Protocol::Quake2_34 {
                    90.
                } else {
                    1.
                },
            ),
            (
                &b"\\name\\player\\fov\\-20"[..],
                if protocol == Protocol::Quake2_34 {
                    90.
                } else {
                    1.
                },
            ),
            (&b"\\name\\player\\fov\\180"[..], 160.),
        ] {
            host.runtime
                .network
                .get_mut(client, Endpoint::Client)
                .unwrap()
                .channel
                .queue_userinfo(protocol, text)
                .unwrap();
            run_client_frames(&mut host, &mut source, 3);
            assert_eq!(
                host.runtime.server.clients[client.0 as usize]
                    .player
                    .view
                    .fov_x,
                Some(expected),
                "{protocol:?} {text:?}"
            );
            assert_eq!(host.runtime.prediction[0].player.view.fov_x, Some(expected));
        }
        let gravity = host
            .console
            .cvars
            .bind("g_gravity", Context::default())
            .unwrap();
        host.console.cvars.force_write(gravity, "701").unwrap();
        run_client_frames(&mut host, &mut source, 3);
        // The SERVER's independently accepted 180 text is deliberately unlike
        // the producer cvar text; an unrelated write must not queue it again.
        let accepted = host.runtime.server.userinfo(client).unwrap();
        let range = qa_core::text::info_value(accepted, b"fov", false).unwrap();
        assert_eq!(&accepted[range], b"180");
        let context = Context {
            source: protocol.rule_set(),
            ..Default::default()
        };
        let fov = host.console.cvars.bind("fov", context).unwrap();
        host.console.cvars.force_write(fov, "117").unwrap();
        run_client_frames(&mut host, &mut source, 4);
        assert_eq!(
            host.runtime.server.clients[client.0 as usize]
                .player
                .view
                .fov_x,
            Some(117.)
        );
        assert_eq!(host.runtime.prediction[0].player.view.fov_x, Some(117.));
        assert_eq!(source.view.unwrap().fov_x, 117.);
        let classic = Context {
            source: RuleSetId::Quake2,
            ..Default::default()
        };
        for (name, value) in [("deathmatch", "1"), ("dmflags", "32768")] {
            let binding = host.console.cvars.bind(name, classic).unwrap();
            host.console.cvars.force_write(binding, value).unwrap();
        }
        host.console.cvars.force_write(fov, "130").unwrap();
        run_client_frames(&mut host, &mut source, 4);
        assert_eq!(
            host.runtime.server.clients[client.0 as usize]
                .player
                .view
                .fov_x,
            Some(if protocol == Protocol::Quake2_34 {
                90.
            } else {
                130.
            })
        );
    }
}

#[test]
fn queued_q3_physical_view_height_follows_predicted_crouch_with_independent_trace_rules() {
    use qa_world::collision::{WorldTrace, brushes::BrushTree, surfaces::SurfaceTable};
    let (mut host, mut source, client) = fixture(Protocol::Quake3_68, RuleSetId::Quake3);
    host.runtime.server.clients[client.0 as usize]
        .player
        .movement
        .view_height = 26.;
    snapshot(
        &mut host,
        &mut source,
        client,
        100,
        100,
        Vec3::default(),
        Vec3([0., 0., 64.]),
    );
    let geometry = host
        .runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            vec![],
            SurfaceTable::flags(vec![]),
            BrushTree::direct(0).unwrap(),
            vec![Bounds {
                mins: Vec3([-4096.; 3]),
                maxs: Vec3([4096.; 3]),
            }],
        )
        .unwrap();
    let mut scratch = host.runtime.geometry.scratch();
    let runtime = &mut host.runtime;
    let player = &mut runtime.prediction[0];
    assert!(player.player.view.raw_offset().is_none());
    assert_eq!(player.player.movement.view_height, 26.);
    player.player.prediction = PredictionPolicy::default();
    let mut trace = WorldTrace::new(
        &runtime.geometry,
        geometry,
        0,
        &runtime.server.entities,
        &runtime.server.area,
        &mut scratch,
        CollisionOwner::Lifetime(runtime.server.clients[client.0 as usize].entity),
    );
    player
        .advance(
            UserCmd {
                units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
                duration_ms: 8,
                server_time_ms: 108,
                buttons: buttons::CROUCH,
                ..Default::default()
            },
            &mut trace,
        )
        .unwrap();
    assert!(player.player.movement.ducked);
    assert_eq!(player.player.movement.view_height, 12.);
    assert_eq!(
        (player.player.movement_rules, player.player.trace_rules),
        (RuleSetId::Quake3, RuleSetId::QuakeWorld)
    );
    let expected = player.player.body.position + Vec3([0., 0., 12.]);
    assert_eq!(host.client_views()[0].unwrap().origin, expected);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .movement
            .view_height,
        26.
    );
}
