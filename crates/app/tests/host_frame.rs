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
    primitives::{CommandIntent, ModuleId, PlayerTail, RuleSetId},
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_session::{
    clients::Connection,
    timing::{Tick, TickRate},
};
use std::time::Duration;

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
    fn begin_frame(&mut self) -> EventTime {
        EventTime(self.time * 1_000_000)
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
                    device: DeviceId::Keyboard,
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
    fn wait_time(&mut self, _: Duration) -> EventTime {
        self.waits += 1;
        self.time += 1;
        EventTime(self.time * 1_000_000)
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
fn provider(runtime: &mut Runtime, tick: Tick) {
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
        Runtime::load(std::iter::empty()).unwrap(),
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
fn client_frame_transports_followers_over_predicted_anchors_without_relinking_world() {
    use qa_app::{WorldCollision, client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::primitives::{BodyAttachment, BodyFollow, Bounds, Plane, SurfaceFlags, Vec3};
    use qa_world::collision::{
        Contents,
        brushes::{Brush, BrushTree},
    };
    let mut runtime = Runtime::load(std::iter::empty()).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![Plane {
                normal: Vec3([0.0, 0.0, 1.0]),
                distance: 0.0,
                axis: None,
            }],
            vec![Brush {
                first_plane: 0,
                plane_count: 1,
                contents: Contents::SOLID,
            }],
            vec![SurfaceFlags(0)],
            BrushTree::direct(1).unwrap(),
            vec![Bounds {
                mins: Vec3([-131072.0; 3]),
                maxs: Vec3([131072.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision {
        geometry,
        index: 0,
        scratch: runtime.geometry.scratch(),
    });
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
        let predicted_root = host.runtime.prediction[0].player.body.position;
        assert_eq!(
            host.runtime.prediction[1].player.body.position,
            predicted_root + Vec3([0.0, 128.0, 0.0])
        );
        assert_eq!(
            host.runtime.server.entities.columns.position[child.slot as usize],
            host.runtime.server.clients[0].player.body.position + Vec3([0.0, 128.0, 0.0])
        );
    }
    assert!(
        host.runtime.prediction[0].player.body.position.0[0]
            > host.runtime.server.clients[0].player.body.position.0[0]
    );
    assert_eq!(
        host.runtime.prediction[1].player.movement_rules,
        RuleSetId::Quake
    );
}

#[test]
fn commands_server_second_packets_and_client_share_the_com_frame_path() {
    let mut host = host();
    let id = host
        .runtime
        .server
        .connect(Connection::Local, ModuleId(3), PlayerTail::default(), None)
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
    assert_eq!(host.runtime.network.packets, 2);
    assert_eq!(
        host.runtime.network.last_socket,
        Some(Endpoint::Client.socket())
    );
    assert_eq!(host.runtime.loopback.pending(Endpoint::Client), 0);
    assert_eq!(
        host.runtime.network.last_from,
        Some(qa_network::ingress::Peer::Loopback(
            qa_core::primitives::ClientId(7)
        ))
    );
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .command
            .server_time_ms,
        100
    );
    assert_eq!(source.presents, 2);
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
        fn begin_frame(&mut self) -> EventTime {
            self.now
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
        fn wait_time(&mut self, remaining: Duration) -> EventTime {
            self.waits += 1;
            self.now.0 += remaining.as_nanos() as u64;
            self.now
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
        Runtime::load(std::iter::empty()).unwrap(),
        TickRate::fixed(20).unwrap(),
        vec![],
    )
    .unwrap();
    let bot = host
        .runtime
        .server
        .connect(Connection::Bot, ModuleId(2), PlayerTail::default(), None)
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
