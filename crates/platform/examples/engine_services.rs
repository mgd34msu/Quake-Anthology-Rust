//! Fixed headless module-service workload. This does not qualify gameplay.
use qa_app::Runtime;
use qa_compat::services::{CallContext, ENGINE_CALLS, ServiceStorage};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::ThinkTime;
use qa_core::{
    events::OutputSubmission,
    primitives::{ModuleId, PrintKind},
    sys_events::EventTime,
};
use qa_platform::{
    Stopwatch,
    allocations::{CountingAllocator, begin_frame, end_frame},
};
use qa_world::{
    area::{LinkFlags, LinkOrder},
    entities::AllocationPolicy,
};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::load(4, std::iter::empty()).map_err(|e| e.to_string())?;
    let mut console = Console::new(Context::default())?;
    let mut storage =
        ServiceStorage::load(&[(ModuleId(1), 16), (ModuleId(2), 16)], 8, &console.cvars)
            .map_err(|e| format!("{e:?}"))?;
    let mut scratch = runtime.geometry.scratch();
    let view = console
        .cvars
        .bind("sensitivity", Context::default())
        .ok_or("cvar")?;
    let calls = black_box(&ENGINE_CALLS);
    let mut samples = [0u64; 600];
    let mut checksum = 0u64;
    let mut publications = 0;
    let mut heap = 0;
    let mut bytes = 0;
    begin_frame();
    black_box(vec![0; 32]);
    let positive = end_frame();
    if positive.allocations != 1 {
        return Err("positive control".into());
    }
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        {
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            for i in 0..64 {
                let context = CallContext {
                    module: ModuleId(1 + i % 2),
                    clock: ThinkTime::Seconds(2.0 + frame as f64 + i as f64 / 64.0),
                    console: Context {
                        event_time: Some(EventTime(frame * 1_000_000)),
                        ..Context::default()
                    },
                    allocation: AllocationPolicy::EDICT,
                    link_order: if i % 2 == 0 {
                        LinkOrder::Head
                    } else {
                        LinkOrder::Tail
                    },
                };
                let entity =
                    (calls.spawn)(&mut services, context).map_err(|e| format!("spawn {e:?}"))?;
                (calls.link)(&mut services, context, entity, LinkFlags::SOLID)
                    .map_err(|e| format!("link {e:?}"))?;
                (calls.free)(&mut services, context, entity).map_err(|e| format!("free {e:?}"))?;
                (calls.cvar_set)(&mut services, view, if i % 2 == 0 { "5" } else { "6" })
                    .map_err(|e| format!("cvar {e:?}"))?;
                (calls.configstring)(
                    &mut services,
                    context.module,
                    i as usize % 16,
                    if frame % 2 == 0 {
                        b"\x80even"
                    } else {
                        b"\x81odd"
                    },
                )
                .map_err(|e| format!("config {e:?}"))?;
                checksum = checksum
                    .wrapping_add(entity.slot as u64)
                    .wrapping_add(entity.generation as u64);
                (calls.print)(&mut services, None, PrintKind::Console, b"\x82native\n")
                    .map_err(|e| format!("print {e:?}"))?;
                publications += 1;
            }
            let consumer = services.server.presentation;
            let mut batch = services.server.events.batch(consumer).ok_or("output")?;
            while let Some(event) = services.server.events.next(&mut batch) {
                if !services.server.events.submit(
                    consumer,
                    event.sequence,
                    OutputSubmission::BestEffort,
                ) {
                    return Err("submission".into());
                }
            }
            if !services.server.events.is_empty() {
                return Err("output retention".into());
            }
        }
        let elapsed = timer.elapsed().as_nanos() as u64;
        let count = end_frame();
        if frame >= 60 {
            samples[frame as usize - 60] = elapsed;
            heap += count.allocations + count.reallocations;
            bytes += count.requested_bytes;
        }
    }
    if heap != 0 || bytes != 0 {
        return Err(format!("heap {heap}/{bytes}").into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless typed services, entities, area, cvars, configstrings and output; no gameplay or ABI execution\",\"warmup\":60,\"frames\":600,\"groups_per_frame\":64,\"publications\":{publications},\"checksum\":{checksum},\"heap\":{heap},\"requested_bytes\":{bytes},\"median_ns\":{},\"p99_ns\":{},\"positive_control\":{}}}",
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593],
        positive.allocations
    );
    Ok(())
}
