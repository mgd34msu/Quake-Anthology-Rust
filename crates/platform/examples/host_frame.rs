//! Measured common host path with live event time/UDP and native-rate clocks.
use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource, Provider},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, ModuleId},
    sys_events::{DeviceId, EventKind, SysEventQueue},
};
use qa_input::Input;
use qa_platform::{EventPump, Stopwatch};
use qa_session::timing::{Tick, TickRate};
use std::{hint::black_box, net::UdpSocket, time::Duration};

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

struct Source {
    pump: EventPump,
    timer: Stopwatch,
    repeats: bool,
}
impl FrameSource for Source {
    fn begin_frame(&mut self, queue: &mut SysEventQueue) {
        self.timer = Stopwatch::start();
        let _ = self.pump.enqueue(
            queue,
            EventKind::Key {
                device: DeviceId::Keyboard,
                code: 26,
                symbol: 119,
                down: true,
                repeat: self.repeats,
            },
        );
        self.repeats = true;
        self.poll_events(queue);
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.pump.poll_network(queue);
        let _ = self.pump.enqueue(queue, EventKind::Time);
    }
    fn wait_events(&mut self, queue: &mut SysEventQueue, remaining: Duration) {
        qa_platform::pause(remaining.min(Duration::from_millis(2)));
        self.poll_events(queue);
    }
    fn elapsed(&self) -> Duration {
        self.timer.elapsed()
    }
    fn present(&mut self) {}
}
fn provider(runtime: &mut Runtime, tick: Tick) {
    if let qa_session::timing::TickTarget::Provider(module) = tick.target {
        runtime.server.clients[module.0 as usize].player.score += 1;
        let _ = runtime.loopback.send(
            Endpoint::Server,
            ClientId(module.0 as u8),
            b"provider packet",
        );
    }
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{begin_frame, end_frame};
    let local = std::env::args().nth(1).as_deref() == Some("--local");
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
        Input::load(),
        Runtime::load()?,
        TickRate::fixed(50).ok_or("world rate")?,
        providers,
    )?;
    let mut samples = [0u64; 600];
    let mut maximum = 0;
    let mut maximum_bytes = 0;
    let mut ticks = 0;
    for frame in 0..660 {
        // Pacing is outside the timed/counted region, but supplies actual
        // platform event time to exercise all loaded native-rate clocks.
        qa_platform::pause(Duration::from_millis(16));
        if let Some((sender, address)) = &udp {
            sender.send_to(b"host packet", address)?;
        }
        host.console.cvars.reset_lookup_count();
        begin_frame();
        if local {
            host.runtime
                .loopback
                .send(Endpoint::Client, ClientId(1), b"host packet")
                .map_err(|_| "local send")?;
        }
        let result = host.frame(&mut source, true);
        let counts = end_frame();
        if result.drains != 2 || !host.queue.is_empty() || host.console.cvars.lookup_count() != 0 {
            return Err("host drain/lookup check failed".into());
        }
        ticks += result.server_ticks;
        black_box(result.commands);
        if frame >= 60 {
            samples[frame - 60] = result.total_ns;
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
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless Com_Frame, time, key repeats, native provider counters and same-frame local snapshots; no gameplay\",\"local_client_packets\":{local},\"warmup\":60,\"frames\":600,\"drains_per_frame\":2,\"packets\":{},\"repeats\":{},\"world_q2_rr_q3_ticks\":{native:?},\"maximum_allocations\":{maximum},\"maximum_requested_bytes\":{maximum_bytes},\"median_ns\":{},\"p99_ns\":{}}}",
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
