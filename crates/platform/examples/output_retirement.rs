//! Fixed-capacity host output with an unacknowledged peer beside a healthy peer.
use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource, Provider},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{events::*, primitives::*, sys_events::*};
use qa_platform::{
    Stopwatch,
    allocations::{CountingAllocator, begin_frame, end_frame},
};
use qa_session::{
    clients::Connection,
    timing::{Tick, TickRate},
};
use std::time::Duration;
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
struct Source {
    frame: u64,
    healthy: u64,
    stalled: u64,
    sounds: u64,
    stale: u64,
    unsent: bool,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> EventTime {
        EventTime(self.frame * 25_000_000)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        let _ = queue.push(SysEvent {
            time: self.begin_frame(),
            kind: EventKind::Time,
        });
    }
    fn wait_time(&mut self, _: Duration) -> EventTime {
        self.begin_frame()
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, _: SoundEvent) -> bool {
        self.sounds += 1;
        true
    }
    fn output(
        &mut self,
        client: ClientId,
        event: FrameEvent,
        text: Option<&[u8]>,
    ) -> OutputSubmission {
        if matches!(event, FrameEvent::Print(_)) && text.is_none() {
            self.stale += 1;
        }
        if client == ClientId(0) {
            self.stalled += 1;
            if self.unsent {
                OutputSubmission::Unsent
            } else {
                OutputSubmission::Reliable(NativeReceipt(7))
            }
        } else {
            self.healthy += 1;
            OutputSubmission::BestEffort
        }
    }
}
fn emit(runtime: &mut Runtime, tick: Tick) {
    runtime.print_event(
        None,
        PrintKind::Center,
        format_args!("{}:{}", tick.source_slot, tick.index),
    );
    let _ = runtime.server.events.push(FrameEvent::Sound(SoundEvent {
        sound: SoundId(tick.source_slot as u32),
        entity: None,
        channel: 1,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 1.0,
        action: SoundAction::Play,
    }));
}
fn consume(runtime: &mut Runtime, tick: Tick, record: OutputRecord) -> OutputSubmission {
    let player = &mut runtime.server.clients[10 + tick.source_slot].player;
    if let FrameEvent::Print(p) = record.event
        && runtime.server.events.texts.get(p.text).is_none()
    {
        player.armor += 1;
    }
    player.health += 1;
    OutputSubmission::BestEffort
}
fn run(unsent: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::load(std::iter::empty())?;
    runtime.server.events = EventRing::load(32, 8, 64, 256).map_err(|_| "events")?;
    runtime.server.presentation = runtime
        .server
        .events
        .bind(OutputTarget::Presentation)
        .ok_or("presentation")?;
    for connection in [Connection::Remote, Connection::Remote, Connection::Local] {
        runtime
            .server
            .connect(connection, ModuleId(0), PlayerTail::None, None)
            .ok_or("clients")?;
    }
    let stalled = runtime.server.clients[0].output.ok_or("stalled cursor")?;
    let healthy = runtime.server.clients[1].output.ok_or("healthy cursor")?;
    let mut host = FrameHost::load(
        Console::new(Context::default()).map_err(|e| e.to_string())?,
        runtime,
        TickRate::fixed(25).ok_or("world rate")?,
        [100, 50, 25]
            .into_iter()
            .enumerate()
            .map(|(slot, ms)| Provider {
                module: ModuleId(slot as u16 + 1),
                rate: TickRate::fixed(ms).expect("fixed rate"),
                frame: emit,
                output: Some(consume),
            })
            .collect(),
    )?;
    host.local_clients[0] = Some(ClientId(2));
    let mut source = Source {
        frame: 0,
        healthy: 0,
        stalled: 0,
        sounds: 0,
        stale: 0,
        unsent,
    };
    // Positive control and all load-time allocation are outside measured frames.
    begin_frame();
    std::hint::black_box(vec![1u8; 32]);
    if end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut samples = [0u64; 600];
    let mut maximum = 0;
    let mut bytes = 0;
    let mut ticks = 0;
    let mut leased_id = None;
    let mut resync_frame = None;
    let mut continuing_frames = 0;
    for frame in 0..660 {
        source.frame = frame;
        let healthy_before = source.healthy;
        let sounds_before = source.sounds;
        begin_frame();
        let timer = Stopwatch::start();
        let result = host.frame(&mut source, true);
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if result.drains != 2 || result.output_drains != 1 {
            return Err("host drain phases".into());
        }
        // Check each complete SERVER -> CLIENT cycle, including the precise
        // overflow frame. Aggregate totals alone could hide a publication wait.
        if frame > 0 {
            let produced = 2 * (result.server_ticks - 1);
            if host.runtime.server.world_frame != frame
                || source.healthy - healthy_before != produced
                || source.sounds - sounds_before != produced / 2
                || produced < 2
            {
                return Err(format!("peer stalled host progress at frame {frame}").into());
            }
            if frame >= 60 {
                continuing_frames += 1;
            }
        }
        if host.runtime.server.events.needs_resync(stalled) {
            resync_frame.get_or_insert(frame);
        }
        let line = host.runtime.server.clients[2].hud.centerprint.as_ref();
        if let Some(line) = line {
            if host
                .runtime
                .server
                .events
                .texts
                .get(line.text.id())
                .is_none()
            {
                return Err("HUD lease expired".into());
            }
            leased_id = Some(line.text.id());
        }
        if frame >= 60 {
            samples[frame as usize - 60] = elapsed;
            maximum = maximum.max(counts.allocations + counts.reallocations);
            bytes = bytes.max(counts.requested_bytes);
            ticks += result.server_ticks;
        }
    }
    let counters = host
        .runtime
        .server
        .events
        .counters(stalled)
        .ok_or("stalled counters")?;
    let healthy_counters = host
        .runtime
        .server
        .events
        .counters(healthy)
        .ok_or("healthy counters")?;
    let modules: [i32; 3] =
        std::array::from_fn(|slot| host.runtime.server.clients[11 + slot].player.health);
    if source.healthy != 2304
        || (!unsent && source.stalled != 30)
        || source.stalled == 0
        || source.sounds != 1152
        || source.stale != 0
        || counters.acknowledgements != 0
        || counters.overflow != 1
        || counters.resyncs != 1
        || counters.retired_on_resync != 32
        || counters.skipped_during_resync != 2272
        || healthy_counters.overflow != 0
        || maximum != 0
        || bytes != 0
        || host.runtime.server.world_frame != 659
        || modules != [2292, 2300, 2304]
        || continuing_frames != 600
        || resync_frame.is_none_or(|frame| frame >= 60)
        || host.runtime.server.clients[11..14]
            .iter()
            .any(|c| c.player.armor != 0)
    {
        return Err(format!("retirement fidelity: healthy={} stalled={} sounds={} counters={counters:?} modules={modules:?} allocations={maximum}",
            source.healthy, source.stalled, source.sounds).into());
    }
    if host
        .runtime
        .server
        .events
        .acknowledge(stalled, NativeReceipt(7))
        != 0
    {
        return Err("stale ACK".into());
    }
    if !host.runtime.server.disconnect(ClientId(2)) {
        return Err("disconnect".into());
    }
    // Module slots may still retain the last message; disconnect must at least
    // release every HUD display lease, independently of module delivery.
    let display_after_disconnect =
        leased_id.is_some_and(|id| host.runtime.server.events.texts.get(id).is_some());
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless Com_Frame output retirement; modeled receipts, no native channel or gameplay\",\"stalled_delivery\":\"{}\",\"warmup\":60,\"frames\":600,\"continuous_healthy_and_server_frames\":{continuing_frames},\"resync_frame\":{},\"server_ticks\":{ticks},\"world_frame\":659,\"module_hz\":[10,20,40],\"module_deliveries\":{modules:?},\"healthy_deliveries\":{},\"stalled_attempts\":{},\"stalled_acks\":0,\"retired_on_resync\":32,\"skipped_during_resync\":2272,\"stalled_overflow\":1,\"stalled_resyncs\":1,\"healthy_overflow\":0,\"stale_texts\":0,\"payload_retained_for_slower_module_after_hud_disconnect\":{display_after_disconnect},\"maximum_allocations\":{maximum},\"maximum_requested_bytes\":{bytes},\"median_ns\":{},\"p99_ns\":{}}}",
        if unsent {
            "unsent"
        } else {
            "reliable_without_ack"
        },
        resync_frame.ok_or("missing resync")?,
        source.healthy,
        source.stalled,
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593]
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for unsent in [false, true] {
        run(unsent)?;
    }
    Ok(())
}
