use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource, Provider},
};
use qa_console::{
    command_text::Arguments,
    commands::{CommandError, Console},
    views::Context,
};
use qa_core::{
    loopback::Endpoint,
    primitives::{CommandIntent, ModuleId, RuleSetId},
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_session::{
    clients::Connection,
    timing::{Tick, TickRate},
};
use std::time::Duration;

#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;

#[derive(Default)]
struct Source {
    time: u64,
    waits: u64,
    polls: u64,
    presents: u64,
    late_commands: bool,
    wait_key: bool,
}
impl Source {
    fn stamp(&self, queue: &mut SysEventQueue, kind: EventKind<'_>) {
        queue
            .push(SysEvent {
                time: EventTime(self.time * 1_000_000),
                kind,
            })
            .unwrap();
    }
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time * 1_000_000)
    }
    #[expect(
        clippy::manual_is_multiple_of,
        reason = "The fixture selects the second physical intake with explicit poll parity"
    )]
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        assert!(queue.is_empty());
        self.polls += 1;
        if self.late_commands && self.polls % 2 == 0 {
            self.stamp(queue, EventKind::ConsoleLine("after_server"));
            self.stamp(
                queue,
                EventKind::Packet {
                    socket: 0,
                    from: "127.0.0.1:1234"
                        .parse::<std::net::SocketAddr>()
                        .unwrap()
                        .into(),
                    bytes: b"late packet",
                },
            );
        }
        if self.wait_key && self.waits >= 2 {
            self.wait_key = false;
            self.stamp(
                queue,
                EventKind::Key {
                    device: DeviceId::Keyboard(0),
                    code: 26,
                    symbol: 119,
                    down: true,
                    repeat: false,
                },
            );
            self.stamp(
                queue,
                EventKind::Packet {
                    socket: 0,
                    from: "127.0.0.1:1234"
                        .parse::<std::net::SocketAddr>()
                        .unwrap()
                        .into(),
                    bytes: b"packet arrived during cap wait",
                },
            );
        }
        self.stamp(queue, EventKind::Time);
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        self.waits += 1;
        self.time += 1;
        Duration::from_nanos(self.time * 1_000_000)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {
        self.presents += 1;
    }
}

fn before(
    _: &mut Console<Runtime>,
    runtime: &mut Runtime,
    _: &Arguments<'_>,
    _: Context,
) -> Result<(), CommandError> {
    runtime.server.clients[0].player.health = 10;
    Ok(())
}
fn provider(host: &mut FrameHost, tick: Tick) {
    let runtime = &mut host.runtime;
    assert!(runtime.server.clients[0].player.health >= 10);
    runtime.server.clients[0].player.health = 20;
    runtime.server.clients[tick.index as usize].player.score += 1;
    runtime
        .loopback
        .send(
            Endpoint::Server,
            qa_core::primitives::ClientId(7),
            b"same-frame snapshot",
        )
        .unwrap();
}

#[test]
fn provider_commands_use_the_host_console_before_client_phase() {
    fn frame(host: &mut FrameHost, _: Tick) {
        host.console
            .append_line("sensitivity 5 extra", Context::default())
            .unwrap();
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![Provider {
            module: ModuleId(1),
            rate: TickRate::FrameDriven,
            frame,
            output: None,
        }],
    )
    .unwrap();
    let mut source = Source::default();
    host.frame(&mut source, true);
    source.time = 20;
    host.frame(&mut source, true);
    let handle = host.console.cvars.find("sensitivity").unwrap();
    assert_eq!(host.console.cvars.value(handle), 5.0);
}
fn after(
    _: &mut Console<Runtime>,
    runtime: &mut Runtime,
    _: &Arguments<'_>,
    _: Context,
) -> Result<(), CommandError> {
    assert_eq!(runtime.server.clients[0].player.health, 20);
    runtime.server.clients[0].player.health = 30;
    Ok(())
}

fn host() -> FrameHost {
    let mut console = Console::new(Context::default()).unwrap();
    console.register("before_server", before);
    console.register("after_server", after);
    FrameHost::load(
        console,
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::fixed(50).unwrap(),
        vec![Provider {
            module: ModuleId(2),
            rate: TickRate::fixed(100).unwrap(),
            frame: provider,
            output: None,
        }],
    )
    .unwrap()
}

#[test]
fn native_player_values_reach_client_and_hud_through_the_queued_snapshot() {
    use qa_app::{client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::primitives::NumericValue;
    use qa_network::commands::packet::Protocol;
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut runtime = Runtime::load(2, []).unwrap();
        // Protocol, presentation, movement and trace are independent choices.
        let policy = ClientPolicy::select(
            Some(RuleSetId::Quake),
            None,
            Some(RuleSetId::QuakeWorld),
            Some(RuleSetId::Quake3),
        )
        .unwrap();
        let id = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Default::default(),
                    angles: Default::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                policy,
                protocol,
                0,
            )
            .unwrap();
        let wire_rules = qa_network::states::player_value_layout(protocol).0;
        let bindings = runtime.catalog.hud.values.native(wire_rules);
        let bank_row = bindings.stats[2];
        let other_row = runtime.catalog.hud.values.native(RuleSetId::Quake).stats[2];
        let player = &mut runtime.server.clients[id.0 as usize].player;
        player.health = 71;
        player.armor = 29;
        player.frags = -3;
        player.score = -5;
        assert!(bank_row.import(&mut player.values, (-123i32) as u32));
        assert!(other_row.import(&mut player.values, 999));
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::fixed(50).unwrap(),
            vec![],
        )
        .unwrap();
        host.local_clients[0] = Some(id);
        let mut source = Source::default();
        for ms in [0, 100, 200] {
            source.time = ms;
            assert_eq!(host.frame(&mut source, true).drains, 2);
        }
        if protocol != Protocol::Quake3_68 {
            assert_eq!(
                host.runtime.prediction[0].packet_acknowledged(),
                Some(qa_core::command_history::Acknowledgement::Number(2))
            );
        }
        let player = &host.runtime.prediction[0].player;
        assert_eq!((player.health, player.armor), (71, 29));
        assert_eq!(
            player.values.get(bank_row.id),
            Some(NumericValue::integer(-123))
        );
        assert_eq!(
            player.values.get(other_row.id),
            Some(NumericValue::integer(0))
        );
        assert_eq!(player.movement_rules, RuleSetId::QuakeWorld);
        assert_eq!(player.trace_rules, RuleSetId::Quake3);
        let client = &host.runtime.server.clients[id.0 as usize];
        assert_eq!(client.client_rules, RuleSetId::Quake);
        assert_eq!((client.hud.health, client.hud.armor), (71, 29));
        assert_eq!(
            client.hud.values.get(bank_row.id),
            Some(NumericValue::integer(-123))
        );
        assert_eq!(
            client.hud.values.get(other_row.id),
            Some(NumericValue::integer(0))
        );
        assert!(
            host.runtime.snapshots[id.0 as usize]
                .as_ref()
                .unwrap()
                .applied
                > 0
        );
    }
}

#[test]
fn registered_q3_player_values_reach_client_and_hud_through_session_packets() {
    use qa_app::{
        client_policy::ClientPolicy,
        map::SpawnAnchor,
        modules::{Export, ModuleRequest, Phase, Program},
    };
    use qa_compat::{abi::Q3_SERVER, services::CallContext};
    use qa_core::primitives::{CallbackId, NativeEntity, NumericValue, ThinkTime};
    use qa_formats::program::qvm::Opcode::*;
    use qa_network::commands::packet::Protocol;
    use qa_world::{area::LinkOrder, entities::AllocationPolicy};
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime
        .server
        .connect(Connection::Remote, ModuleId(2), None)
        .unwrap();
    let id = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(
                Some(RuleSetId::Quake),
                None,
                Some(RuleSetId::QuakeWorld),
                Some(RuleSetId::Quake2),
            )
            .unwrap(),
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    assert_eq!(id.0, 1);
    assert!(runtime.server.bind_client_native(
        id,
        NativeEntity {
            module: ModuleId(1),
            slot: 0
        }
    ));
    let entity_base = 4096;
    let player_base = 16384;
    let mut data = vec![0; player_base + 512];
    let stats = std::array::from_fn::<_, 16, _>(|i| match i {
        0 => 71,
        3 => 29,
        14 => i32::MIN,
        15 => i32::MAX,
        _ => -123 - i as i32,
    });
    let persistent =
        std::array::from_fn::<_, 16, _>(|i| if i == 0 { -5 } else { 90_000 + i as i32 });
    for (offset, values) in [(184, stats), (248, persistent)] {
        for (i, value) in values.into_iter().enumerate() {
            let at = player_base + offset + i * 4;
            data[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    let mut vm = service_program::program(
        &[
            (Enter, 64),
            (Const, entity_base as i32),
            (Arg, 8),
            (Const, 1),
            (Arg, 12),
            (Const, 608),
            (Arg, 16),
            (Const, player_base as i32),
            (Arg, 20),
            (Const, 512),
            (Arg, 24),
            (Const, -16),
            (Call, 0),
            (Pop, 0),
            (Const, 0),
            (Leave, 64),
        ],
        &data,
    );
    vm.configure_q3_game(qa_core::primitives::Q3StatLayout::BASE);
    let module = ModuleId(1);
    let allocation = AllocationPolicy::q3(0);
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), module, allocation)
        .unwrap()
        .id;
    let request = ModuleRequest {
        context: CallContext {
            module,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::Quake2,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake),
            server_frame: 0,
            clock: ThinkTime::Milliseconds(0),
            console: Context::default(),
            allocation,
            link_order: LinkOrder::Head,
        },
        phase: Phase::Server,
        timing_rules: RuleSetId::Quake3,
        rate: TickRate::fixed(50).unwrap(),
        anchor,
        program: Program::Qvm {
            vm: Box::new(vm),
            imports: &Q3_SERVER,
        },
        entries: vec![0],
        frame: Export::clocked(CallbackId(0)),
        prepare: vec![],
        initialize: vec![Export::clocked(CallbackId(0))],
        api: None,
        shutdown: vec![],
        instruction_budget: 100,
        configstrings: 0,
        files: 0,
        entity_source: None,
    };
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(50).unwrap(),
        vec![request],
    )
    .unwrap();
    host.local_clients[0] = Some(id);
    host.initialize_modules(Phase::Server);
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
    assert!(host.client_info(module).is_none());
    assert_eq!(
        host.module_state(module),
        Some(qa_app::modules::State::Running)
    );
    let bindings = host.runtime.catalog.hud.values.native(RuleSetId::Quake3);
    for (array, expected) in [(&bindings.stats, stats), (&bindings.persistent, persistent)] {
        for (binding, value) in array.iter().zip(expected) {
            assert_eq!(
                host.runtime.server.clients[id.0 as usize]
                    .player
                    .values
                    .get(binding.id),
                Some(NumericValue::integer(value))
            );
        }
    }
    let mut source = Source::default();
    for ms in [0, 100, 200] {
        source.time = ms;
        assert_eq!(host.frame(&mut source, true).drains, 2);
    }
    let client = &host.runtime.server.clients[id.0 as usize];
    let prediction = &host.runtime.prediction[0].player;
    let bindings = host.runtime.catalog.hud.values.native(RuleSetId::Quake3);
    for (array, expected) in [(&bindings.stats, stats), (&bindings.persistent, persistent)] {
        for (binding, value) in array.iter().zip(expected) {
            // Original Q3 signed16 arrays narrow only at the native wire boundary.
            let received = NumericValue::integer(value as i16 as i32);
            assert_eq!(prediction.values.get(binding.id), Some(received));
            assert_eq!(client.hud.values.get(binding.id), Some(received));
            assert_eq!(
                client.player.values.get(binding.id),
                Some(NumericValue::integer(value))
            );
        }
    }
    assert_eq!(
        (prediction.health, prediction.armor, prediction.score),
        (71, 29, -5)
    );
    assert_eq!(
        (client.hud.health, client.hud.armor, client.hud.score),
        (71, 29, -5)
    );
    assert_eq!(
        (prediction.movement_rules, prediction.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    assert_eq!(client.client_rules, RuleSetId::Quake);
    assert!(!client.module_movement);
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
    assert!(
        host.runtime.snapshots[id.0 as usize]
            .as_ref()
            .unwrap()
            .applied
            > 0
    );
}

#[test]
fn untransmitted_server_attachments_do_not_modify_client_prediction() {
    use qa_app::{WorldCollision, client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::primitives::{BodyAttachment, BodyFollow, Bounds, Plane, SurfaceFlags, Vec3};
    use qa_world::collision::{
        Contents,
        brushes::{Brush, BrushTree},
    };
    let mut runtime = Runtime::load(64, std::iter::empty()).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![Plane {
                encoding: None,
                normal: Vec3([0.0, 0.0, 1.0]),
                distance: 0.0,
                axis: None,
            }],
            vec![Brush {
                sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                contents: Contents::SOLID,
            }],
            qa_world::collision::surfaces::SurfaceTable::flags(vec![SurfaceFlags(0)]),
            BrushTree::direct(1).unwrap(),
            vec![Bounds {
                mins: Vec3([-131072.0; 3]),
                maxs: Vec3([131072.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    let mut local = [None; SeatId::COUNT];
    for (seat, rules, y) in [
        (SeatId::FIRST, RuleSetId::Quake3, 0.0),
        (SeatId::new(1).unwrap(), RuleSetId::Quake, 128.0),
    ] {
        let policy =
            ClientPolicy::select(Some(rules), None, Some(rules), Some(RuleSetId::Quake3)).unwrap();
        let client = runtime
            .connect_local(
                seat,
                SpawnAnchor {
                    position: Vec3([0.0, y, 24.125]),
                    angles: Vec3::default(),
                    entity: seat.index() + 1,
                    fixture_fallback: false,
                },
                policy,
                qa_network::commands::packet::Protocol::QuakeWorld28,
                seat.index() as u32,
            )
            .unwrap();
        runtime.server.clients[client.0 as usize]
            .player
            .movement
            .grounded = true;
        local[seat.index()] = Some(client);
    }
    let root = runtime.server.clients[0].entity;
    let child = runtime.server.clients[1].entity;
    runtime
        .server
        .entities
        .attach(
            child,
            BodyAttachment {
                anchor: root,
                follow: BodyFollow::Translation,
                offset: Vec3([0.0, 128.0, 0.0]),
            },
        )
        .unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    console
        .append_line("bind w +forward", Context::default())
        .unwrap();
    let mut host = FrameHost::load(console, runtime, TickRate::FrameDriven, vec![]).unwrap();
    host.local_clients = local;
    let client_positions = [Vec3([40.0, -200.0, 300.0]), Vec3([-20.0, 10.0, 80.0])];
    for (seat, position) in client_positions.into_iter().enumerate() {
        host.runtime.prediction[seat].player.body.position = position;
        // These native protocols have no attachment graph field. Keep the
        // CLIENT sample fixed while the separate SERVER graph advances.
        host.runtime.snapshots[local[seat].unwrap().0 as usize] = None;
    }
    // The source's queued-key fixture injects at the first physical intake.
    let mut source = Source {
        time: 0,
        waits: 2,
        polls: 0,
        presents: 0,
        late_commands: false,
        wait_key: true,
    };
    for ms in [0, 16, 32] {
        source.time = ms;
        let result = host.frame(&mut source, true);
        assert_eq!(result.drains, 2);
        for (seat, position) in client_positions.into_iter().enumerate() {
            assert_eq!(host.runtime.prediction[seat].player.body.position, position);
            let id = local[seat].unwrap();
            assert!(matches!(
                host.runtime.client_collision[id.0 as usize]
                    .as_mut()
                    .unwrap()
                    .trace(&host.runtime.geometry),
                Err(qa_app::client_collision::ClientCollisionError::Waiting)
            ));
        }
        assert_eq!(
            host.runtime.server.entities.columns.position[child.slot as usize],
            host.runtime.server.clients[0].player.body.position + Vec3([0.0, 128.0, 0.0])
        );
    }
    assert!(host.runtime.server.clients[0].player.body.position.0[0] > 0.0);
    assert_eq!(
        host.runtime.prediction[1].player.movement_rules,
        RuleSetId::Quake
    );
}

#[test]
fn server_set_angles_survive_q2_and_q3_client_prediction() {
    use qa_app::{WorldCollision, client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::primitives::{Bounds, Vec3};
    use qa_network::commands::packet::Protocol;
    use qa_world::collision::{brushes::BrushTree, surfaces::SurfaceTable};

    let mut runtime = Runtime::load(64, []).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            SurfaceTable::flags(vec![]),
            BrushTree::direct(0).unwrap(),
            vec![Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    let mut local = [None; SeatId::COUNT];
    let server_angles = [Vec3([22.5, 90.0, 0.0]), Vec3([-22.5, 135.0, 0.0])];
    for (seat, rules, protocol) in [
        (SeatId::FIRST, RuleSetId::Quake2, Protocol::Quake2_34),
        (
            SeatId::new(1).unwrap(),
            RuleSetId::Quake3,
            Protocol::Quake3_68,
        ),
    ] {
        let client = runtime
            .connect_local(
                seat,
                SpawnAnchor {
                    position: Vec3([0.0, seat.index() as f32 * 128.0, 64.0]),
                    angles: Vec3::default(),
                    entity: seat.index() + 1,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(rules), None, None, None).unwrap(),
                protocol,
                seat.index() as u32,
            )
            .unwrap();
        let player = &mut runtime.server.clients[client.0 as usize].player;
        // Native teleport/set-angle adjusts delta_angles relative to the
        // client's unchanged raw command angles. Deliver it in a snapshot.
        player.movement.delta_angles = server_angles[seat.index()].into();
        player.view_angles = server_angles[seat.index()];
        local[seat.index()] = Some(client);
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients = local;
    for seat in 0..2 {
        host.local_worlds[seat] = Some((
            qa_render::world::WorldId(0),
            qa_render::world::WorldPresentationId(seat as u32),
        ));
    }
    let mut source = Source::default();
    for ms in [100, 116, 132] {
        source.time = ms;
        let frame = host.frame(&mut source, true);
        assert_eq!(frame.drains, 2);
        for (seat, angles) in server_angles.iter().enumerate() {
            let client = local[seat].unwrap();
            assert!(
                host.runtime.snapshots[client.0 as usize]
                    .as_ref()
                    .unwrap()
                    .applied
                    > 0
            );
            assert_eq!(frame.commands[seat].view_angles, Vec3::default());
            assert_eq!(host.runtime.prediction[seat].player.view_angles, *angles);
            assert_eq!(host.client_views()[seat].unwrap().angles, *angles);
            assert_eq!(
                host.client_views()[seat].unwrap().world_presentation,
                qa_render::world::WorldPresentationId(seat as u32)
            );
        }
    }
}

#[test]
fn commands_server_second_packets_and_client_share_the_com_frame_path() {
    let mut host = host();
    let id = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            qa_app::map::SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            qa_app::client_policy::ClientPolicy::select(
                Some(RuleSetId::QuakeWorld),
                None,
                None,
                None,
            )
            .unwrap(),
            qa_network::commands::packet::Protocol::QuakeWorld28,
            0,
        )
        .unwrap();
    host.local_clients[SeatId::FIRST.index()] = Some(id);
    let mut source = Source {
        time: 0,
        waits: 0,
        polls: 0,
        presents: 0,
        late_commands: false,
        wait_key: false,
    };
    host.frame(&mut source, true);
    host.console
        .append("before_server\n", Context::default())
        .unwrap();
    source.time = 100;
    source.late_commands = true;
    let frame = host.frame(&mut source, true);
    assert_eq!(frame.drains, 2);
    assert_eq!(frame.server_ticks, 3); // two world ticks and one Q2 provider tick
    assert_eq!(host.runtime.server.world_frame, 2);
    assert_eq!(host.runtime.server.clients[0].player.health, 30);
    assert_eq!(host.runtime.network.packets, 4); // includes the admitted SERVER snapshot
    assert_eq!(
        host.runtime.network.last_socket,
        Some(Endpoint::Client.socket())
    );
    assert_eq!(host.runtime.loopback.pending(Endpoint::Client), 0);
    assert_eq!(
        host.runtime.network.last_from,
        Some(qa_network::ingress::Peer::Loopback(id))
    );
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .command
            .server_time_ms,
        0
    );
    assert_eq!(host.runtime.loopback.pending(Endpoint::Server), 1);
    source.late_commands = false;
    source.time = 200;
    host.frame(&mut source, true);
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .command
            .server_time_ms,
        100
    );
    assert_eq!(source.presents, 3);
    assert!(host.queue.is_empty());
}

#[test]
fn cap_wait_has_no_intake_and_aliases_use_one_cached_fps_handle() {
    let mut host = host();
    let cap = host.console.cvars.find("com_maxfps").unwrap();
    host.console.cvars.set(cap, 100.0).unwrap();
    let mut source = Source {
        time: 0,
        waits: 0,
        polls: 0,
        presents: 0,
        late_commands: false,
        wait_key: true,
    };
    let frame = host.frame(&mut source, false);
    assert_eq!(source.waits, 10);
    assert_eq!(frame.drains, 2);
    assert_eq!(source.polls, 2);
    assert_eq!(host.key_downs, 1);
    // The first physical poll timestamps the press at the initial command's
    // endpoint; the following frame measures the held interval.
    assert_eq!(frame.commands[0].movement, [0.0; 3]);
    assert_eq!(frame.commands[1].movement, [0.0; 3]);
    assert_eq!(host.runtime.network.packets, 1);
    let alias = host.console.cvars.find("cl_maxfps").unwrap();
    assert_eq!(alias, cap);
    host.console.cvars.set(alias, 200.0).unwrap();
    source.wait_key = false;
    let frame = host.frame(&mut source, false);
    assert_eq!(source.waits, 15);
    assert_eq!(frame.drains, 2);
    assert_eq!(source.polls, 4);
    assert!(frame.commands[0].movement[0] > 0.0);
    // Native integer division produces no delay above 1000 fps. The explicit
    // uncapped path also skips waiting while preserving both intake points.
    host.console.cvars.set(cap, 2001.0).unwrap();
    host.frame(&mut source, false);
    host.console.cvars.set(cap, 85.0).unwrap();
    host.frame(&mut source, true);
    assert_eq!(source.waits, 15);
    assert_eq!(source.polls, 8);
    assert!(host.queue.is_empty());
}

#[test]
fn cap_uses_native_millisecond_timestamps_and_zero_startup_baseline() {
    struct ClockSource {
        now: EventTime,
        polls: usize,
        waits: usize,
    }
    impl FrameSource for ClockSource {
        fn begin_frame(&mut self) -> Duration {
            Duration::from_nanos(self.now.0)
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            self.polls += 1;
            queue
                .push(SysEvent {
                    time: self.now,
                    kind: EventKind::Time,
                })
                .unwrap();
        }
        fn wait_time(&mut self, remaining: Duration) -> Duration {
            self.waits += 1;
            self.now.0 += remaining.as_nanos() as u64;
            Duration::from_nanos(self.now.0)
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
    }
    let mut host = host();
    let mut source = ClockSource {
        now: EventTime(5_000_000_000),
        polls: 0,
        waits: 0,
    };
    host.frame(&mut source, false);
    assert_eq!(source.waits, 0);
    assert_eq!(source.polls, 2);

    // Native 5011-5000 is 11 ms even though the precise duration is 10.002 ms.
    source.now = EventTime(5_000_999_000);
    host.frame(&mut source, true);
    source.now = EventTime(5_011_001_000);
    let frame = host.frame(&mut source, false);
    assert_eq!(source.waits, 0);
    assert_eq!(frame.drains, 2);
    assert_eq!(source.polls, 6);

    // At the next deadline only 9.1 ms remains, rather than a fresh 11 ms.
    source.now = EventTime(5_012_900_000);
    host.frame(&mut source, false);
    assert_eq!(source.now, EventTime(5_022_000_000));
    assert_eq!(source.waits, 1);
    assert_eq!(source.polls, 8);
}

#[test]
fn startup_epoch_and_world_ticks_keep_bot_commands_out_of_client_frames() {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::fixed(20).unwrap(),
        vec![],
    )
    .unwrap();
    let bot = host
        .runtime
        .server
        .connect(Connection::Bot, ModuleId(2), None)
        .unwrap();
    host.runtime.server.clients[bot.0 as usize]
        .player
        .movement_rules = RuleSetId::Quake2;
    host.runtime.server.clients[bot.0 as usize].intent = CommandIntent::moving([0.15, -0.1, 0.05]);
    let mut source = Source {
        time: 5_000,
        waits: 0,
        polls: 0,
        presents: 0,
        late_commands: false,
        wait_key: false,
    };
    let first = host.frame(&mut source, true);
    assert_eq!(first.server_ticks, 0);
    assert_eq!(first.commands[0].duration_ms, 1);
    assert_eq!(first.commands[0].server_time_ms, 5_000);
    assert_eq!(
        host.runtime.server.clients[bot.0 as usize]
            .command
            .duration_ms,
        0
    );
    source.time += 25;
    let next = host.frame(&mut source, true);
    assert_eq!(next.commands[0].duration_ms, 25);
    let command = host.runtime.server.clients[bot.0 as usize].command;
    assert_eq!(command.duration_ms, 20); // 50 Hz world, independent of client frame
    assert_eq!(command.server_time_ms, 5_020);
    assert_eq!(command.movement, [30.0, -20.0, 10.0]);
    source.time += 5;
    let next = host.frame(&mut source, true);
    assert_eq!(next.commands[0].duration_ms, 5);
    assert_eq!(
        host.runtime.server.clients[bot.0 as usize]
            .command
            .server_time_ms,
        5_020
    );
    assert_eq!(
        host.runtime.server.clients[bot.0 as usize].command.movement,
        [30.0, -20.0, 10.0]
    );
}

#[test]
fn large_local_messages_use_queued_event_dispatch_without_an_extra_intake() {
    let mut host = host();
    let mut source = Source {
        time: 0,
        waits: 0,
        polls: 0,
        presents: 0,
        late_commands: false,
        wait_key: false,
    };
    host.runtime
        .loopback
        .send(
            Endpoint::Server,
            qa_core::primitives::ClientId(0),
            &[137; 8000],
        )
        .unwrap();
    host.runtime
        .loopback
        .send(
            Endpoint::Client,
            qa_core::primitives::ClientId(1),
            &[255; 64000],
        )
        .unwrap();
    let frame = host.frame(&mut source, true);
    assert_eq!(source.polls, 2);
    assert_eq!(frame.drains, 2);
    assert_eq!(frame.events, 4); // two Time markers and two queued local packets
    assert_eq!(host.runtime.network.packets, 2);
    assert_eq!(host.runtime.network.bytes, 72000);
    assert_eq!(
        host.runtime.network.last_socket,
        Some(Endpoint::Server.socket())
    );
    assert_eq!(
        host.runtime.network.last_from,
        Some(qa_network::ingress::Peer::Loopback(
            qa_core::primitives::ClientId(1)
        ))
    );
    assert_eq!(host.runtime.loopback.pending(Endpoint::Client), 0);
    assert_eq!(host.runtime.loopback.pending(Endpoint::Server), 0);
    assert!(host.queue.is_empty());
}
