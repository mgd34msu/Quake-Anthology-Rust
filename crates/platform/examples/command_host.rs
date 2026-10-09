//! Four independent native move transports through the ordinary host queue.
use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::RuleSetId,
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_network::commands::packet::Protocol;
use qa_platform::{Stopwatch, allocations};
use qa_session::timing::TickRate;
use std::time::Duration;

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

struct Source {
    frame: u64,
    polls: u64,
    presents: u64,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> EventTime {
        EventTime(self.frame * 16_000_000)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        let time = EventTime(self.frame * 16_000_000);
        if self.polls & 1 == 0 {
            for seat in 0..4 {
                let _ = queue.push(SysEvent {
                    time,
                    kind: EventKind::ControllerAxis {
                        device: DeviceId::Controller(100 + seat),
                        axis: 1,
                        value: -16000 - seat as i16 * 2000,
                    },
                });
            }
        }
        self.polls += 1;
        let _ = queue.push(SysEvent {
            time,
            kind: EventKind::Time,
        });
    }
    fn wait_time(&mut self, _: Duration) -> EventTime {
        EventTime(self.frame * 16_000_000)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {
        self.presents += 1;
    }
}
fn main() -> Result<(), String> {
    let mut runtime = Runtime::load(4, [])?;
    let protocols = [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake3_68,
    ];
    let roles = [
        RuleSetId::Quake3,
        RuleSetId::Quake,
        RuleSetId::Quake2,
        RuleSetId::QuakeWorld,
    ];
    let mut locals = [None; SeatId::COUNT];
    for seat in SeatId::ALL {
        let slot = seat.index();
        let policy = ClientPolicy::select(Some(roles[slot]), None, None, None)?;
        let id = runtime.connect_local(
            seat,
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            policy,
            protocols[slot],
        )?;
        locals[slot] = Some(id);
        runtime
            .input
            .assign(DeviceId::Controller(100 + slot as i32), seat);
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).map_err(|e| e.to_string())?,
        runtime,
        TickRate::FrameDriven,
        vec![],
    )?;
    host.local_clients = locals;
    let mut source = Source {
        frame: 0,
        polls: 0,
        presents: 0,
    };
    let mut samples = [0u64; 600];
    let mut totals = allocations::Counts::default();
    let mut ticks = 0;
    allocations::begin_frame();
    let positive = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&positive);
    drop(positive);
    if allocations::end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    for frame in 0..660 {
        source.frame = frame;
        allocations::begin_frame();
        let watch = Stopwatch::start();
        let result = host.frame(&mut source, true);
        let elapsed = watch.elapsed().as_nanos() as u64;
        let counts = allocations::end_frame();
        if result.drains != 2
            || result.output_drains != 1
            || host.runtime.network.command_errors != 0
        {
            return Err("host phase or native parse fidelity".into());
        }
        if frame >= 60 {
            samples[(frame - 60) as usize] = elapsed;
            ticks += result.server_ticks;
            totals.allocations += counts.allocations;
            totals.reallocations += counts.reallocations;
            totals.requested_bytes += counts.requested_bytes;
            for client in &host.runtime.server.clients {
                if client.command.duration_ms != 16 || client.command.movement[0] <= 0. {
                    return Err("native movement fields".into());
                }
            }
        }
    }
    if host.runtime.network.commands != 659 * 4 - 1
        || source.polls != 660 * 2
        || source.presents != 660
    {
        return Err("host packet fixture counts".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"ordinary headless host with four independently selected native local move protocols; no map or signon\",\"warmup\":60,\"frames\":600,\"protocols\":[15,28,34,68],\"packets\":{},\"decoded_commands\":{},\"physical_intake_calls\":{},\"measured_ticks\":{ticks},\"median_ns\":{},\"p99_ns\":{},\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{}}}",
        host.runtime.network.packets,
        host.runtime.network.commands,
        source.polls,
        (samples[299] + samples[300]) / 2,
        samples[593],
        totals.allocations,
        totals.reallocations,
        totals.requested_bytes
    );
    if totals.allocations + totals.reallocations != 0 {
        return Err("measured Rust heap activity".into());
    }
    Ok(())
}
