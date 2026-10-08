#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), String> {
    use qa_core::primitives::Vec3;
    use qa_platform::Stopwatch;
    use qa_platform::allocations::{begin_frame, end_frame};
    use std::hint::black_box;
    // Prove the counter catches actual heap activity before using its zero result.
    begin_frame();
    let mut positive = Vec::with_capacity(4);
    positive.extend_from_slice(&[1u64; 4]);
    positive.reserve_exact(32);
    black_box(&positive);
    let caught = end_frame();
    if caught.allocations != 1 || caught.reallocations != 1 || caught.requested_bytes != 320 {
        return Err(format!("positive control: {caught:?}"));
    }
    let mut points = vec![Vec3::default(); 1024].into_boxed_slice();
    let mut bytes = vec![0u8; 4096].into_boxed_slice();
    let mut scratch = Vec::with_capacity(256);
    let mut maximum = 0;
    let mut samples = [0u64; 600];
    let mut allocation_calls = 0;
    let mut reallocation_calls = 0;
    let mut requested_bytes = 0;
    let _ = Stopwatch::start().elapsed();
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        points.fill(Vec3::default());
        bytes.fill(0);
        points[1023].0[0] = 17.0;
        black_box(&points);
        black_box(&bytes);
        scratch.clear();
        scratch.extend(0..256u32);
        black_box(&scratch);
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            maximum = maximum.max(counts.allocations + counts.reallocations);
            allocation_calls += counts.allocations;
            reallocation_calls += counts.reallocations;
            requested_bytes += counts.requested_bytes;
        }
    }
    if maximum != 0 {
        return Err(format!("owned scratch loop allocated: {maximum}"));
    }
    samples.sort_unstable();
    let median = (samples[299] as f64 + samples[300] as f64) * 0.5;
    let p99 = samples[593];
    println!(
        "{{\"scope\":\"headless owned fixed scratch and allocation counter; no gameplay or workers\",\"warmup\":60,\"measured_frames\":600,\"points\":1024,\"bytes\":4096,\"scratch_values\":256,\"median_ns\":{median},\"p99_ns\":{p99},\"positive_allocations\":{},\"positive_reallocations\":{},\"positive_requested_bytes\":{},\"maximum_allocations\":{maximum},\"rust_allocations\":{allocation_calls},\"rust_reallocations\":{reallocation_calls},\"rust_requested_bytes\":{requested_bytes}}}",
        caught.allocations, caught.reallocations, caught.requested_bytes
    );
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    println!("Use a debug build or --features qa-platform/allocation-tracking");
    std::process::exit(1);
}
