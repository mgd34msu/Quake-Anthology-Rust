use qa_core::primitives::{Axis, Plane, Vec3};
use qa_world::collision::{
    Contents,
    hulls::{ClipNode, HullModel, Q1Hulls},
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{hint::black_box, time::Instant};

struct Counter;
static MEASURING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if MEASURING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if MEASURING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(pointer, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counter = Counter;

fn word(data: &mut &[u8]) -> Result<u32, &'static str> {
    let (head, tail) = data.split_first_chunk::<4>().ok_or("truncated workload")?;
    *data = tail;
    Ok(u32::from_le_bytes(*head))
}
fn vector(data: &mut &[u8]) -> Result<Vec3, &'static str> {
    Ok(Vec3([
        f32::from_bits(word(data)?),
        f32::from_bits(word(data)?),
        f32::from_bits(word(data)?),
    ]))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("expected segments and C results".into());
    }
    let payload = std::fs::read(&args[1])?;
    let mut data = payload.as_slice();
    let counts = [
        word(&mut data)? as usize,
        word(&mut data)? as usize,
        word(&mut data)? as usize,
    ];
    let roots = [
        word(&mut data)? as i32,
        word(&mut data)? as i32,
        word(&mut data)? as i32,
    ];
    let mut planes = Vec::with_capacity(counts[0]);
    for _ in 0..counts[0] {
        let normal = vector(&mut data)?;
        let distance = f32::from_bits(word(&mut data)?);
        let axis = match word(&mut data)? {
            0 => Some(Axis::X),
            1 => Some(Axis::Y),
            2 => Some(Axis::Z),
            _ => None,
        };
        planes.push(Plane {
            normal,
            distance,
            axis,
        });
    }
    let mut node_sets = [Vec::with_capacity(counts[1]), Vec::with_capacity(counts[2])];
    for (nodes, &count) in node_sets.iter_mut().zip(&counts[1..]) {
        for _ in 0..count {
            nodes.push(ClipNode {
                plane: word(&mut data)?,
                children: [word(&mut data)? as i32, word(&mut data)? as i32],
            });
        }
    }
    let [drawing, clips] = node_sets;
    let mut hulls = Q1Hulls::load(planes, drawing, clips, vec![HullModel { roots }])
        .map_err(|err| format!("hull load: {err:?}"))?;
    let mut queries = Vec::new();
    while !data.is_empty() {
        queries.push((word(&mut data)?, vector(&mut data)?, vector(&mut data)?));
    }
    let references = std::fs::read(&args[2])?;
    if references.len() != queries.len() * 36 {
        return Err("reference count differs".into());
    }
    let bounds = [
        (Vec3::default(), Vec3::default()),
        (Vec3([-16.0, -16.0, -24.0]), Vec3([16.0, 16.0, 32.0])),
        (Vec3([-32.0, -32.0, -24.0]), Vec3([32.0, 32.0, 64.0])),
    ];
    for (index, (&(hull, start, end), expected)) in queries
        .iter()
        .zip(references.as_chunks::<36>().0)
        .enumerate()
    {
        let (mins, maxs) = bounds[hull as usize];
        let trace = hulls.trace(start, end, mins, maxs, Contents::SOLID);
        let flags = u32::from(trace.start_solid)
            | u32::from(trace.all_solid) << 1
            | u32::from(trace.in_open) << 2
            | u32::from(trace.in_water) << 3;
        let actual = [
            trace.fraction.to_bits(),
            trace.end.0[0].to_bits(),
            trace.end.0[1].to_bits(),
            trace.end.0[2].to_bits(),
            trace.plane.normal.0[0].to_bits(),
            trace.plane.normal.0[1].to_bits(),
            trace.plane.normal.0[2].to_bits(),
            trace.plane.distance.to_bits(),
            flags,
        ];
        let reference: [u32; 9] = std::array::from_fn(|column| {
            u32::from_le_bytes(expected[column * 4..column * 4 + 4].try_into().unwrap())
        });
        if actual != reference {
            return Err(
                format!("C mismatch at segment {index}: {actual:x?} != {reference:x?}").into(),
            );
        }
    }
    for &(hull, start, end) in queries.iter().take(600) {
        let (mins, maxs) = bounds[hull as usize];
        black_box(hulls.trace(start, end, mins, maxs, Contents::SOLID));
    }
    let mut samples = vec![0u128; queries.len() * 60];
    MEASURING.store(true, Ordering::Relaxed);
    for (sample, &(hull, start, end)) in samples.iter_mut().zip(queries.iter().cycle()) {
        let (mins, maxs) = bounds[hull as usize];
        let started = Instant::now();
        black_box(hulls.trace(
            black_box(start),
            black_box(end),
            mins,
            maxs,
            Contents::SOLID,
        ));
        *sample = started.elapsed().as_nanos();
    }
    MEASURING.store(false, Ordering::Relaxed);
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    samples.sort_unstable();
    println!(
        "{{\"bit_exact_segments\":{},\"warmup_traces\":600,\"timed_traces\":{},\"median_ns\":{},\"p99_ns\":{},\"allocations_after_load\":{allocations}}}",
        queries.len(),
        samples.len(),
        samples[samples.len() / 2],
        samples[samples.len() * 99 / 100]
    );
    if allocations != 0 {
        return Err("trace allocated".into());
    }
    Ok(())
}
