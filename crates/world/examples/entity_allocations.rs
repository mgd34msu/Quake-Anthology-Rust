use qa_core::primitives::ModuleId;
use qa_world::entities::EntityTable;
use std::alloc::{GlobalAlloc, Layout, System};
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
    let mut table = EntityTable::new(512, 2).map_err(|_| "capacity")?;
    let mut ids = [None; 256];
    let mut operations = 0;
    MEASURING.store(true, Ordering::Relaxed);
    for step in 0..10_000 {
        let now = 10.0 + f64::from(step);
        for entry in &mut ids {
            *entry = Some(table.allocate(now, ModuleId(1)).ok_or("full table")?);
            operations += 1;
        }
        for entry in &mut ids {
            if let Some(id) = entry.take()
                && !table.release(id, now)
            {
                return Err("stale handle");
            }
        }
    }
    MEASURING.store(false, Ordering::Relaxed);
    let count = ALLOCATIONS.load(Ordering::Relaxed);
    println!(
        "{{\"scope\":\"headless entity allocation workload; no gameplay\",\"cycles\":10000,\"allocation_operations\":{operations},\"allocations_after_load\":{count},\"remaining_entities\":{}}}",
        table.len()
    );
    if count == 0 && table.len() == 2 {
        Ok(())
    } else {
        Err("allocation or lifecycle mismatch")
    }
}
