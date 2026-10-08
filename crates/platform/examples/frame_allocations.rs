#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), String> {
    use qa_core::{arena::FrameArena, primitives::Vec3};
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
    let mut arena = FrameArena::load(65536).map_err(|e| format!("{e:?}"))?;
    let mut scratch = Vec::with_capacity(256);
    let mut maximum = 0;
    for _ in 0..10000 {
        begin_frame();
        arena.reset().map_err(|e| format!("{e:?}"))?;
        let points = arena
            .allocate(1024, Vec3::default())
            .map_err(|e| format!("{e:?}"))?;
        let bytes = arena.allocate(4096, 0u8).map_err(|e| format!("{e:?}"))?;
        let point_slice = arena.get_mut(points).ok_or("point handle")?;
        point_slice[1023].0[0] = 17.0;
        black_box(point_slice);
        black_box(arena.get(bytes).ok_or("byte handle")?);
        scratch.clear();
        scratch.extend(0..256u32);
        black_box(&scratch);
        let counts = end_frame();
        maximum = maximum.max(counts.allocations + counts.reallocations);
    }
    if maximum != 0 {
        return Err(format!("arena/scratch loop allocated: {maximum}"));
    }
    println!(
        "{{\"scope\":\"headless arena and owned scratch, not gameplay\",\"iterations\":10000,\"positive_allocations\":{},\"positive_reallocations\":{},\"positive_requested_bytes\":{},\"maximum_allocations\":{maximum}}}",
        caught.allocations, caught.reallocations, caught.requested_bytes
    );
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    println!("Use a debug build or --features qa-platform/allocation-tracking");
    std::process::exit(1);
}
