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
    primitives::{CommandIntent, ModuleId, MovementRules, PlayerTail},
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
    fn begin_frame(&mut self, queue: &mut SysEventQueue) {
        assert!(queue.is_empty());
        self.stamp(queue, EventKind::Time);
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        if self.late_commands {
            self.stamp(queue, EventKind::ConsoleLine("after_server"));
            self.stamp(
                queue,
                EventKind::Packet {
                    socket: 0,
                    from: "127.0.0.1:1234".parse().unwrap(),
                    bytes: b"late packet",
                },
            );
        }
        self.stamp(queue, EventKind::Time);
    }
    fn wait_events(&mut self, queue: &mut SysEventQueue, _: Duration) {
        assert!(queue.is_empty());
        self.waits += 1;
        self.time += 1;
        if self.wait_key && self.waits == 2 {
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
        }
        self.stamp(queue, EventKind::Time);
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
    let mut console = Console::new(Context::default());
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
        }],
    )
    .unwrap()
}

#[test]
fn commands_server_second_packets_and_client_share_the_com_frame_path() {
    let mut host = host();
    let id = host
        .runtime
        .server
        .connect(Connection::Local, ModuleId(3), PlayerTail::default())
        .unwrap();
    host.local_clients[SeatId::FIRST.index()] = Some(id);
    let mut source = Source {
        time: 0,
        waits: 0,
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
fn cap_wait_drains_keys_and_aliases_use_one_cached_fps_handle() {
    let mut host = host();
    let cap = host.console.cvars.find("com_maxfps").unwrap();
    host.console.cvars.set(cap, 100.0).unwrap();
    let mut source = Source {
        time: 0,
        waits: 0,
        presents: 0,
        late_commands: false,
        wait_key: true,
    };
    let frame = host.frame(&mut source, false);
    assert_eq!(source.waits, 10);
    assert_eq!(frame.drains, 12);
    assert_eq!(host.key_downs, 1);
    assert!(frame.commands[0].movement[0] > 0);
    assert_eq!(frame.commands[1].movement, [0; 3]);
    let alias = host.console.cvars.find("cl_maxfps").unwrap();
    assert_eq!(alias, cap);
    host.console.cvars.set(alias, 200.0).unwrap();
    source.wait_key = false;
    host.frame(&mut source, false);
    assert_eq!(source.waits, 15);
    assert!(host.queue.is_empty());
}

#[test]
fn startup_epoch_and_world_ticks_keep_bot_commands_out_of_client_frames() {
    let mut host = FrameHost::load(
        Console::new(Context::default()),
        Runtime::load(std::iter::empty()).unwrap(),
        TickRate::fixed(20).unwrap(),
        vec![],
    )
    .unwrap();
    let bot = host
        .runtime
        .server
        .connect(Connection::Bot, ModuleId(2), PlayerTail::default())
        .unwrap();
    host.runtime.server.clients[bot.0 as usize]
        .player
        .movement_rules = MovementRules::Quake2;
    host.runtime.server.clients[bot.0 as usize].intent = CommandIntent {
        movement: [30, -20, 10],
        ..CommandIntent::default()
    };
    let mut source = Source {
        time: 5_000,
        waits: 0,
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
    assert_eq!(command.movement, [30, -20, 10]);
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
        [30, -20, 10]
    );
}
