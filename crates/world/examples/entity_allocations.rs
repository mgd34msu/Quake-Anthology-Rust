use qa_core::names::NameTable;
use qa_core::primitives::ModuleId;
use qa_world::entities::EntityTable;
use qa_world::targets::TargetIndex;
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
    let names = NameTable::load([b"door".as_slice(), b"exit"]).map_err(|_| "name capacity")?;
    let door = names.find(b"DOOR").ok_or("missing door")?;
    let exit = names.find(b"exit").ok_or("missing exit")?;
    let mut targets = TargetIndex::new(&table);
    let mut ids = [None; 256];
    let mut operations = 0;
    MEASURING.store(true, Ordering::Relaxed);
    for step in 0..10_000 {
        let now = 10.0 + f64::from(step);
        for (index, entry) in ids.iter_mut().enumerate() {
            let id = table.allocate(now, ModuleId(1)).ok_or("full table")?;
            table.set_targetname(id, if index % 2 == 0 { door } else { exit });
            *entry = Some(id);
            operations += 1;
        }
        targets.refresh(&table);
        if targets.find(door).count() != 128 || targets.refresh(&table) {
            return Err("target index mismatch");
        }
        for entry in &mut ids {
            if let Some(id) = entry.take()
                && !table.release(id, now)
            {
                return Err("stale handle");
            }
        }
        targets.refresh(&table);
        if targets.find(door).next().is_some() {
            return Err("freed target remains");
        }
    }
    MEASURING.store(false, Ordering::Relaxed);
    let count = ALLOCATIONS.load(Ordering::Relaxed);
    println!(
        "{{\"scope\":\"headless entity and target-index workload; no gameplay\",\"cycles\":10000,\"allocation_operations\":{operations},\"allocations_after_load\":{count},\"remaining_entities\":{}}}",
        table.len()
    );
    if count == 0 && table.len() == 2 {
        Ok(())
    } else {
        Err("allocation or lifecycle mismatch")
    }
}
