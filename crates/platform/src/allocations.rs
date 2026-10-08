//! Development-only counts for the calling Rust thread, excluding reporting.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub allocations: u64,
    pub reallocations: u64,
    pub requested_bytes: u64,
}

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<Counts> = const { Cell::new(Counts {
        allocations: 0, reallocations: 0, requested_bytes: 0
    }) };
}

fn record(bytes: usize, reallocation: bool) {
    let _ = ACTIVE.try_with(|active| {
        if active.get() {
            let _ = COUNTS.try_with(|counts| {
                let mut value = counts.get();
                if reallocation {
                    value.reallocations = value.reallocations.saturating_add(1);
                } else {
                    value.allocations = value.allocations.saturating_add(1);
                }
                value.requested_bytes = value.requested_bytes.saturating_add(bytes as u64);
                counts.set(value);
            });
        }
    });
}

pub fn begin_frame() {
    COUNTS.with(|counts| counts.set(Counts::default()));
    ACTIVE.with(|active| active.set(true));
}

pub fn end_frame() -> Counts {
    ACTIVE.with(|active| active.set(false));
    COUNTS.with(Cell::get)
}

pub struct CountingAllocator;

// SAFETY: every allocation operation delegates with unchanged pointer/layout
// arguments to System. The thread-local counters use no heap allocation.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size(), false);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size(), false);
        }
        pointer
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(pointer, layout, size) };
        if !new.is_null() {
            record(size, true);
        }
        new
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}
