use qa_core::primitives::{Body, EntityId, Vec3};
use qa_world::collision::{Contents, TraceQuery, TraceRules, boxes::trace_box};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

fn main() -> Result<(), &'static str> {
    let bodies: [Body; 50] = std::array::from_fn(|index| Body {
        position: Vec3([index as f32 * 64.0, 0.0, 0.0]),
        mins: Vec3([-16.0, -16.0, -24.0]),
        maxs: Vec3([16.0, 16.0, 32.0]),
        ..Body::default()
    });
    let mut hits = 0;
    MEASURING.store(true, Ordering::Relaxed);
    for _ in 0..1000 {
        for (slot, body) in bodies.iter().enumerate() {
            let start = Vec3([body.position.0[0] + 100.0, 0.0, 0.0]);
            let trace = trace_box(
                TraceQuery {
                    start: black_box(start),
                    end: body.position,
                    mins: Vec3([-16.0, -16.0, -24.0]),
                    maxs: Vec3([16.0, 16.0, 32.0]),
                    mask: Contents::SOLID,
                    rules: TraceRules::LEGACY,
                },
                body,
                EntityId {
                    slot: slot as u32,
                    generation: 1,
                },
            );
            if black_box(trace).fraction < 1.0 {
                hits += 1;
            }
        }
    }
    MEASURING.store(false, Ordering::Relaxed);
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    println!(
        "{{\"scope\":\"headless box hull workload; no gameplay\",\"queries\":1000,\"entities_per_query\":50,\"box_traces\":50000,\"hits\":{hits},\"allocations\":{allocations}}}"
    );
    if hits == 50_000 && allocations == 0 {
        Ok(())
    } else {
        Err("allocation or hit mismatch")
    }
}
