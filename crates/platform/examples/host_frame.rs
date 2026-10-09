//! Measured common host path with live event time/UDP and native-rate clocks.
use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource, Provider},
};
use qa_console::{
    commands::Console,
    views::{Context, RuleSetId as CommandSource},
};
use qa_core::{
    events::FrameEvent,
    loopback::Endpoint,
    primitives::{
        ClientId, CommandIntent, EffectEvent, EffectId, ModuleId, PlayerTail, PrintKind, RuleSetId,
        SoundAction, SoundEvent, SoundId, Vec3, WeaponId, buttons,
    },
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_platform::{EventPump, Stopwatch};
use qa_session::{
    clients::Connection,
    timing::{Tick, TickRate},
};
use std::{hint::black_box, net::UdpSocket, time::Duration};

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

struct Source {
    pump: EventPump,
    timer: Stopwatch,
    repeats: bool,
    console: bool,
    binds: bool,
    fixture_frame: u64,
    sounds: u64,
    effects: u64,
    first_poll: bool,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> EventTime {
        self.timer = Stopwatch::start();
        self.first_poll = true;
        let now = self.pump.begin_frame();
        if self.console {
            EventTime(self.fixture_frame * 16_000_000)
        } else {
            now
        }
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        if self.first_poll {
            self.first_poll = false;
            self.input(queue);
        }
        self.pump.poll_console(queue);
        self.pump.poll_network(queue);
        if self.console {
            let _ = queue.push(SysEvent {
                time: EventTime(self.fixture_frame * 16_000_000),
                kind: EventKind::Time,
            });
        } else {
            let _ = self.pump.enqueue(queue, EventKind::Time);
        }
    }
    fn wait_time(&mut self, remaining: Duration) -> EventTime {
        self.pump.wait_time(remaining)
    }
    fn elapsed(&self) -> Duration {
        self.timer.elapsed()
    }
    fn present(&mut self) {}
    fn sound(&mut self, event: SoundEvent) -> bool {
        self.sounds += 1;
        (1..=3).contains(&event.sound.0) && event.channel == 1 && event.volume == 1.0
    }
    fn effect(&mut self, event: EffectEvent) -> bool {
        self.effects += 1;
        (1..=3).contains(&event.effect.0) && event.count == 20
    }
}
impl Source {
    fn input(&mut self, queue: &mut SysEventQueue) {
        let key = EventKind::Key {
            device: DeviceId::Keyboard,
            code: 26,
            symbol: 119,
            down: true,
            repeat: self.repeats,
        };
        if self.console {
            let time = EventTime(self.fixture_frame * 16_000_000);
            let _ = queue.push(SysEvent { time, kind: key });
            let _ = queue.push(SysEvent {
                time,
                kind: EventKind::ConsoleLine("echo queue"),
            });
            if self.binds {
                let down = self.fixture_frame & 1 == 0;
                let _ = queue.push(SysEvent {
                    time,
                    kind: EventKind::Key {
                        device: DeviceId::Keyboard,
                        code: 10,
                        symbol: 103,
                        down,
                        repeat: false,
                    },
                });
                let _ = queue.push(SysEvent {
                    time,
                    kind: EventKind::Key {
                        device: DeviceId::Keyboard,
                        code: 11,
                        symbol: 104,
                        down,
                        repeat: false,
                    },
                });
                let _ = queue.push(SysEvent {
                    time,
                    kind: EventKind::ControllerButton {
                        device: DeviceId::Controller(42),
                        button: 0,
                        down,
                    },
                });
            }
        } else {
            let _ = self.pump.enqueue(queue, key);
        }
        self.repeats = true;
    }
}
fn provider(runtime: &mut Runtime, tick: Tick) {
    if let qa_session::timing::TickTarget::Provider(module) = tick.target {
        runtime.server.clients[module.0 as usize].player.score += 1;
        let _ = runtime.loopback.send(
            Endpoint::Server,
            ClientId(u32::from(module.0)),
            b"provider packet",
        );
    }
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{begin_frame, end_frame};
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let local = arguments.iter().any(|s| s == "--local");
    let binds = arguments.iter().any(|s| s == "--binds");
    let bots = arguments.iter().any(|s| s == "--bots");
    let outputs = arguments.iter().any(|s| s == "--outputs");
    let console = bots || binds || arguments.iter().any(|s| s == "--console");
    let content = arguments
        .windows(2)
        .find(|s| s[0] == "--content")
        .map(|s| &s[1]);
    if arguments.iter().any(|s| s == "--help") {
        println!(
            "host_frame [--local] [--console | --binds] [--bots] [--outputs] [--content DIRECTORY]"
        );
        return Ok(());
    }
    begin_frame();
    let mut positive = Vec::with_capacity(4);
    positive.extend_from_slice(&[1u64; 4]);
    positive.reserve_exact(32);
    black_box(&positive);
    let positive = end_frame();
    if positive.allocations != 1 || positive.reallocations != 1 {
        return Err("allocator positive control failed".into());
    }
    let mut pump = EventPump::new();
    let udp = if local {
        None
    } else {
        let (_, address) = pump.bind_udp("127.0.0.1:0".parse()?)?;
        Some((UdpSocket::bind("127.0.0.1:0")?, address))
    };
    let mut source = Source {
        pump,
        timer: Stopwatch::start(),
        repeats: false,
        console,
        binds,
        fixture_frame: 0,
        sounds: 0,
        effects: 0,
        first_poll: false,
    };
    let providers = [(1, 100), (2, 25), (3, 50)]
        .map(|(id, ms)| {
            Ok(Provider {
                module: ModuleId(id),
                rate: TickRate::fixed(ms).ok_or("rate")?,
                frame: provider,
            })
        })
        .into_iter()
        .collect::<Result<Vec<_>, &str>>()?;
    let mut host = FrameHost::load(
        Console::new(Context::default()),
        Runtime::load(std::iter::empty())?,
        TickRate::fixed(50).ok_or("world rate")?,
        providers,
    )?;
    if let Some(root) = content {
        host.runtime
            .vfs
            .mount_product(std::path::Path::new(root), 0)
            .map_err(|_| "content mount")?;
    }
    if console && content.is_none() {
        return Err("--console requires --content with inner.cfg".into());
    }
    if bots {
        let rules = [
            RuleSetId::Quake,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake2,
            RuleSetId::Quake2Rerelease,
            RuleSetId::Quake3,
        ];
        for slot in 0..64 {
            let id = host
                .runtime
                .server
                .connect(
                    Connection::Bot,
                    ModuleId((slot % 3 + 1) as u16),
                    PlayerTail::default(),
                    None,
                )
                .ok_or("bot capacity")?;
            if id.0 as usize != slot {
                return Err("bot slot ordering".into());
            }
            let client = &mut host.runtime.server.clients[slot];
            client.player.movement_rules = rules[slot % rules.len()];
            client.player.trace_rules = rules[slot % rules.len()];
            client.intent = CommandIntent {
                movement: [slot as i16, -(slot as i16), 17],
                buttons: buttons::ATTACK,
                impulse: slot as u8,
                light_level: 127,
                weapon: Some(WeaponId(3)),
                ..CommandIntent::default()
            };
        }
    }
    if binds {
        host.runtime
            .input
            .assign(DeviceId::Controller(42), SeatId::ALL[1]);
        host.console.append_line("alias +edge +attack; alias -edge -attack; bind w +forward; bind g \"+jump; echo binding\"; bind h +edge; bind JOY1 +moveleft", Context::default()).map_err(|_| "initial binds")?;
        host.console.execute_frame(&mut host.runtime);
    }
    // Resolve the fidelity checks before timing; names are command-boundary
    // lookups inside Console, never lookups in the idle frame consumer.
    let fov = host.console.cvars.find("cg_fov").ok_or("fov")?;
    let sensitivity = host
        .console
        .cvars
        .find("sensitivity")
        .ok_or("sensitivity")?;
    let mut samples = [0u64; 600];
    let mut maximum = 0;
    let mut maximum_bytes = 0;
    let mut ticks = 0;
    let mut bot_commands = 0;
    for frame in 0..660 {
        // Pacing is outside the timed/counted region, but supplies actual
        // platform event time to exercise all loaded native-rate clocks.
        qa_platform::pause(Duration::from_millis(16));
        if let Some((sender, address)) = &udp {
            sender.send_to(b"host packet", address)?;
        }
        source.fixture_frame = frame as u64;
        host.console.cvars.reset_lookup_count();
        begin_frame();
        let timer = Stopwatch::start();
        if console {
            for source in CommandSource::ALL {
                host.console.append("alias timed \"echo alias\"; timed; sensitivity \"echo vstr\"; vstr sensitivity; sensitivity 3; fov 120; gamma 0.8; cl_gun 3; exec inner\n", Context { source, ..Context::default() }).map_err(|_| "console append")?;
            }
            if binds {
                for source in CommandSource::ALL {
                    host.console.append_line("bind w +forward; bind g \"+jump; echo binding\"; bind h +edge; bind JOY1 +moveleft; bind SEMICOLON \"echo semi\"; bind AUX32 +button10; bind MOUSE1 +attack; bind g; unbind UPARROW; bind UPARROW +forward", Context { source, ..Context::default() }).map_err(|_| "config binds")?;
                }
            }
        }
        if local {
            host.runtime
                .loopback
                .send(Endpoint::Client, ClientId(1), b"host packet")
                .map_err(|_| "local send")?;
        }
        if outputs {
            for id in 1..=3 {
                host.runtime.events.push(FrameEvent::Sound(SoundEvent {
                    sound: SoundId(id),
                    entity: None,
                    channel: 1,
                    position: Vec3::default(),
                    volume: 1.0,
                    attenuation: 1.0,
                    action: SoundAction::Play,
                }));
                host.runtime
                    .print_event(None, PrintKind::Console, format_args!("output {id}\n"));
                host.runtime.events.push(FrameEvent::Effect(EffectEvent {
                    effect: EffectId(id),
                    position: Vec3::default(),
                    direction: Vec3::default(),
                    count: 20,
                }));
            }
        }
        let result = host.frame(&mut source, true);
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if result.drains != 2
            || result.output_drains != 1
            || !host.runtime.events.is_empty()
            || (outputs
                && (result.output.sounds != 3
                    || result.output.effects != 3
                    || result.output.unhandled_sounds != 0
                    || result.output.unhandled_effects != 0))
            || !host.queue.is_empty()
            || (!console && host.console.cvars.lookup_count() != 0)
        {
            return Err("host drain/lookup check failed".into());
        }
        ticks += result.server_ticks;
        if bots && host.runtime.server.world_frame > 0 {
            for (slot, client) in host.runtime.server.clients.iter().enumerate() {
                let command = client.command;
                if command.duration_ms != 50
                    || command.server_time_ms
                        != host.runtime.server.world_time.milliseconds() as i32
                    || command.movement != [slot as i16, -(slot as i16), 17]
                    || command.buttons != buttons::ATTACK
                    || command.impulse != slot as u8
                    || command.weapon != Some(WeaponId(3))
                    || command.light_level != 127
                {
                    return Err("server bot command fidelity check failed".into());
                }
            }
        }
        if bots && frame >= 60 {
            // One conversion per connected bot in each world tick.
            let previous_world = ((frame - 1) as u64 * 16) / 50;
            bot_commands += (host.runtime.server.world_frame - previous_world) * 64;
        }
        if binds
            && frame > 0
            && (result.commands[0].movement[0] != 127
                || result.commands[1].movement != [0, if frame & 1 == 1 { -127 } else { 0 }, 0])
        {
            return Err("bind/seat fidelity check failed".into());
        }
        black_box(result.commands);
        if frame >= 60 {
            samples[frame - 60] = if console { elapsed } else { result.total_ns };
            maximum = maximum.max(counts.allocations + counts.reallocations);
            maximum_bytes = maximum_bytes.max(counts.requested_bytes);
        }
    }
    let native = [
        host.runtime.server.world_frame,
        host.runtime.server.clients[1].player.score as u64,
        host.runtime.server.clients[2].player.score as u64,
        host.runtime.server.clients[3].player.score as u64,
    ];
    if host.runtime.network.packets != 660 + native[1..].iter().sum::<u64>()
        || host.key_repeats != 659
        || maximum != 0
        || maximum_bytes != 0
        || native.contains(&0)
        || native[0] != native[3]
        || native.iter().sum::<u64>() != ticks
        || host.runtime.loopback.pending(Endpoint::Client) != 0
        || host.runtime.loopback.pending(Endpoint::Server) != 0
        || host.runtime.loopback.overwritten(Endpoint::Client) != 0
        || host.runtime.loopback.overwritten(Endpoint::Server) != 0
    {
        return Err("host qualification failed".into());
    }
    if console
        && (host.console.cvars.text(fov) != "120"
            || host.console.cvars.text(sensitivity) != "3"
            || !host.console.idle())
    {
        return Err("console fidelity check failed".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless Com_Frame, time, key repeats, native provider counters and same-frame local snapshots; no gameplay\",\"console_workload\":{console},\"binding_workload\":{binds},\"bot_clients\":{},\"measured_bot_commands\":{bot_commands},\"output_workload\":{outputs},\"output_sounds\":{},\"output_effects\":{},\"output_drains_per_frame\":1,\"local_client_packets\":{local},\"warmup\":60,\"frames\":600,\"drains_per_frame\":2,\"packets\":{},\"repeats\":{},\"world_q2_rr_q3_ticks\":{native:?},\"maximum_allocations\":{maximum},\"maximum_requested_bytes\":{maximum_bytes},\"median_ns\":{},\"p99_ns\":{}}}",
        if bots { 64 } else { 0 },
        source.sounds,
        source.effects,
        host.runtime.network.packets,
        host.key_repeats,
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593]
    );
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    println!("Run with allocation-tracking enabled.");
}
