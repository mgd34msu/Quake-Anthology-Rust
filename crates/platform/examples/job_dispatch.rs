//! Pinned developer measurement of the one scoped dispatcher, without SDL.
use qa_platform::{Stopwatch, Workers};
use std::hint::black_box;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

struct Job {
    index: u64,
    iterations: usize,
    result: u64,
    calls: u64,
}

fn work(job: &mut Job) {
    let mut value = job.index;
    for _ in 0..job.iterations {
        value = value.wrapping_mul(3).wrapping_add(17);
    }
    job.result = black_box(value);
    job.calls += 1;
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{Counts, begin_frame, end_frame};
    let mut background = 0usize;
    let mut count = 32usize;
    let mut iterations = 0usize;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().ok_or("option needs a value")?.parse()?;
        match arg.as_str() {
            "--workers" => background = value,
            "--jobs" => count = value,
            "--iterations" => iterations = value,
            _ => return Err("unknown option".into()),
        }
    }
    if count == 0 || count > 65536 || iterations > 1_000_000 {
        return Err("invalid benchmark workload".into());
    }
    begin_frame();
    black_box(vec![17u8; 64]);
    if end_frame().allocations != 1 {
        return Err("allocator positive control failed".into());
    }
    let mut workers = Workers::load(background)?;
    let mut jobs: Vec<_> = (0..count)
        .map(|index| Job {
            index: index as u64,
            iterations,
            result: 0,
            calls: 0,
        })
        .collect();
    let mut worker_counts = vec![Counts::default(); background];
    for _ in 0..60 {
        workers.dispatch_scoped(&mut jobs, work)?;
    }
    let mut samples = [0u64; 600];
    let mut total = Counts::default();
    for sample in &mut samples {
        begin_frame();
        let clock = Stopwatch::start();
        workers.dispatch_scoped(&mut jobs, work)?;
        *sample = clock.elapsed().as_nanos() as u64;
        let caller = end_frame();
        workers.allocation_counts(&mut worker_counts)?;
        for counts in std::iter::once(caller).chain(worker_counts.iter().copied()) {
            total.allocations += counts.allocations;
            total.reallocations += counts.reallocations;
            total.requested_bytes += counts.requested_bytes;
        }
    }
    if total != Counts::default() || jobs.iter().any(|job| job.calls != 660) {
        return Err("dispatch allocation or exact-once gate failed".into());
    }
    let mut sorted = samples;
    sorted.sort_unstable();
    let median = (sorted[299] as f64 + sorted[300] as f64) / 2.0;
    let results: Vec<_> = jobs.iter().map(|job| job.result).collect();
    println!(
        "{{\"event\":\"job_dispatch_timings\",\"background_workers\":{background},\"caller_participates\":true,\"physical_cores\":{},\"jobs\":{count},\"iterations\":{iterations},\"warmup\":60,\"frames\":600,\"median_ns\":{median},\"p99_ns\":{},\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"results\":{results:?},\"samples_ns\":{samples:?}}}",
        qa_platform::physical_core_count(),
        sorted[593]
    );
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    eprintln!("job_dispatch requires allocation-tracking");
}
