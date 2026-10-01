//! Collision statistics from id Software's `code/qcommon/cm_load.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/counters.ts`.
//!
//! Counters are shared across borrows during a trace, so the donor's plain
//! fields become cells with wrapping source arithmetic.

use std::cell::Cell;

/// Common-lived source counters, cleared by `Com_Frame` after
/// `com_showtrace` prints.
#[derive(Debug, Default)]
pub struct CollisionCounters {
    /// `c_traces`: traces started.
    pub c_traces: Cell<i32>,
    /// `c_brush_traces`: brush traces started.
    pub c_brush_traces: Cell<i32>,
    /// `c_patch_traces`: patch traces started.
    pub c_patch_traces: Cell<i32>,
    /// `c_pointcontents`: point queries answered.
    pub c_pointcontents: Cell<i32>,
}

impl CollisionCounters {
    /// Fresh zeroed counters.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all counters.
    pub fn reset(&self) {
        self.c_traces.set(0);
        self.c_brush_traces.set(0);
        self.c_patch_traces.set(0);
        self.c_pointcontents.set(0);
    }

    /// Wrapping `(value + 1) | 0` increment, matching the donor.
    pub fn bump(cell: &Cell<i32>) {
        cell.set(cell.get().wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_all_counters() {
        let counters = CollisionCounters::new();
        CollisionCounters::bump(&counters.c_traces);
        CollisionCounters::bump(&counters.c_brush_traces);
        CollisionCounters::bump(&counters.c_patch_traces);
        CollisionCounters::bump(&counters.c_pointcontents);
        counters.reset();
        assert_eq!(counters.c_traces.get(), 0);
        assert_eq!(counters.c_brush_traces.get(), 0);
        assert_eq!(counters.c_patch_traces.get(), 0);
        assert_eq!(counters.c_pointcontents.get(), 0);
    }

    #[test]
    fn bump_wraps_like_source_int32() {
        let counters = CollisionCounters::new();
        counters.c_traces.set(i32::MAX);
        CollisionCounters::bump(&counters.c_traces);
        assert_eq!(counters.c_traces.get(), i32::MIN);
    }
}
